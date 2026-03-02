use crw_core::config::LlmConfig;
use crw_core::error::CrwResult;
use crw_core::types::{OutputFormat, ScrapeData, ScrapeRequest};
use crw_renderer::FallbackRenderer;
use std::sync::Arc;

/// Scrape a single URL: fetch → extract → (optional) LLM structured extraction.
pub async fn scrape_url(
    req: &ScrapeRequest,
    renderer: &Arc<FallbackRenderer>,
    llm_config: Option<&LlmConfig>,
) -> CrwResult<ScrapeData> {
    let fetch_result = renderer
        .fetch(&req.url, &req.headers, req.render_js, req.wait_for)
        .await?;

    let mut data = crw_extract::extract(
        &fetch_result.html,
        &fetch_result.url,
        fetch_result.status_code,
        fetch_result.rendered_with,
        fetch_result.elapsed_ms,
        &req.formats,
        req.only_main_content,
        &req.include_tags,
        &req.exclude_tags,
    );

    // Phase 4: LLM structured extraction
    if formats_include_json(&req.formats) {
        if let (Some(schema), Some(llm)) = (&req.json_schema, llm_config) {
            let md = data
                .markdown
                .as_deref()
                .unwrap_or("");
            match crw_extract::structured::extract_structured(md, schema, llm).await {
                Ok(json) => data.json = Some(json),
                Err(e) => {
                    tracing::error!("Structured extraction failed: {e}");
                    return Err(e);
                }
            }
        } else if req.json_schema.is_some() && llm_config.is_none() {
            return Err(crw_core::error::CrwError::ExtractionError(
                "JSON extraction requested but no LLM configured. Set [extraction.llm] in config or CRW_EXTRACTION__LLM__API_KEY env var.".into()
            ));
        }
    }

    Ok(data)
}

fn formats_include_json(formats: &[OutputFormat]) -> bool {
    formats.contains(&OutputFormat::Json)
}
