use crw_core::config::{BUILTIN_UA_POOL, LlmConfig};
use crw_core::error::CrwResult;
use crw_core::types::{OutputFormat, ScrapeData, ScrapeRequest};
use crw_renderer::FallbackRenderer;
use crw_renderer::http_only::HttpFetcher;
use crw_renderer::traits::PageFetcher;
use std::sync::Arc;

/// Scrape a single URL: fetch → extract → (optional) LLM structured extraction.
///
/// - `user_agent`: base user-agent string from global config.
/// - `default_stealth`: whether stealth headers are active by global config.
pub async fn scrape_url(
    req: &ScrapeRequest,
    renderer: &Arc<FallbackRenderer>,
    llm_config: Option<&LlmConfig>,
    user_agent: &str,
    default_stealth: bool,
) -> CrwResult<ScrapeData> {
    // Determine whether stealth headers should be injected for this request.
    let inject_stealth = req.stealth.unwrap_or(default_stealth);

    // Use a temporary HttpFetcher when:
    // (a) per-request proxy overrides global proxy, OR
    // (b) per-request stealth differs from what the shared renderer was built with.
    let needs_temp_fetcher =
        req.proxy.is_some() || req.stealth.map_or(false, |s| s != default_stealth);

    let fetch_result = if needs_temp_fetcher {
        let proxy = req.proxy.as_deref();
        // Rotate UA from built-in pool when stealth is active, so the request
        // looks like a real browser even for per-request stealth overrides.
        let effective_ua = if inject_stealth {
            BUILTIN_UA_POOL[rand::random::<usize>() % BUILTIN_UA_POOL.len()].to_string()
        } else {
            user_agent.to_string()
        };
        let temp_http = HttpFetcher::new(&effective_ua, proxy, inject_stealth);
        temp_http
            .fetch(&req.url, &req.headers, req.wait_for)
            .await?
    } else {
        renderer
            .fetch(&req.url, &req.headers, req.render_js, req.wait_for)
            .await?
    };

    let mut data = crw_extract::extract(crw_extract::ExtractOptions {
        raw_html: &fetch_result.html,
        source_url: &fetch_result.url,
        status_code: fetch_result.status_code,
        rendered_with: fetch_result.rendered_with,
        elapsed_ms: fetch_result.elapsed_ms,
        formats: &req.formats,
        only_main_content: req.only_main_content,
        include_tags: &req.include_tags,
        exclude_tags: &req.exclude_tags,
        css_selector: req.css_selector.as_deref(),
        xpath: req.xpath.as_deref(),
        chunk_strategy: req.chunk_strategy.as_ref(),
        query: req.query.as_deref(),
        filter_mode: req.filter_mode.as_ref(),
        top_k: req.top_k,
    });

    // Phase 4: LLM structured extraction
    if formats_include_json(&req.formats) {
        if let (Some(schema), Some(llm)) = (&req.json_schema, llm_config) {
            let md = data.markdown.as_deref().unwrap_or("");
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
