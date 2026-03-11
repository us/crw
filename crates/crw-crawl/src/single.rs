use crw_core::config::{BUILTIN_UA_POOL, LlmConfig};
use crw_core::error::CrwResult;
use crw_core::types::{FetchResult, OutputFormat, ScrapeData, ScrapeRequest};
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
    // Reject unsupported `actions` parameter early with a clear error.
    if req.actions.is_some() {
        return Err(crw_core::error::CrwError::InvalidRequest(
            "The 'actions' parameter is not yet supported. Use cssSelector or xpath for element targeting.".into()
        ));
    }

    // Determine whether stealth headers should be injected for this request.
    let inject_stealth = req.stealth.unwrap_or(default_stealth);

    // Use a temporary HttpFetcher when:
    // (a) per-request proxy overrides global proxy, OR
    // (b) per-request stealth differs from what the shared renderer was built with.
    let needs_temp_fetcher =
        req.proxy.is_some() || req.stealth.is_some_and(|s| s != default_stealth);

    let fetch_result = if needs_temp_fetcher {
        let proxy = req.proxy.as_deref();
        // Rotate UA from built-in pool when stealth is active, so the request
        // looks like a real browser even for per-request stealth overrides.
        let effective_ua = if inject_stealth {
            BUILTIN_UA_POOL[rand::random::<usize>() % BUILTIN_UA_POOL.len()].to_string()
        } else {
            user_agent.to_string()
        };

        if req.render_js == Some(false) {
            // HTTP-only: safe to use a temp HttpFetcher with custom proxy/stealth.
            let temp_http = HttpFetcher::new(&effective_ua, proxy, inject_stealth);
            temp_http
                .fetch(&req.url, &req.headers, req.wait_for)
                .await?
        } else {
            // JS rendering needed (or auto-detect): use the shared renderer which
            // has CDP backends configured. Inject stealth headers via custom headers
            // so the shared renderer's CDP connections are still used.
            let mut merged_headers = req.headers.clone();
            if inject_stealth {
                merged_headers
                    .entry("User-Agent".to_string())
                    .or_insert(effective_ua);
            }
            renderer
                .fetch(&req.url, &merged_headers, req.render_js, req.wait_for)
                .await?
        }
    } else {
        renderer
            .fetch(&req.url, &req.headers, req.render_js, req.wait_for)
            .await?
    };

    let warning = derive_target_warning(&fetch_result);
    let mut data = crw_extract::extract(crw_extract::ExtractOptions {
        raw_html: &fetch_result.html,
        source_url: &fetch_result.url,
        status_code: fetch_result.status_code,
        rendered_with: fetch_result.rendered_with.clone(),
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
    })?;
    data.warning = warning;

    // Phase 4: LLM structured extraction
    if formats_include_json(&req.formats) {
        // Merge per-request LLM config (BYOK) with server config
        let byok_config = req.llm_api_key.as_ref().map(|key| LlmConfig {
            api_key: key.clone(),
            provider: req
                .llm_provider
                .clone()
                .unwrap_or_else(|| "anthropic".into()),
            model: req
                .llm_model
                .clone()
                .unwrap_or_else(|| "claude-sonnet-4-20250514".into()),
            base_url: None,
            max_tokens: 4096,
        });
        let effective_llm = byok_config.as_ref().or(llm_config);

        if let (Some(schema), Some(llm)) = (&req.json_schema, effective_llm) {
            let md = data.markdown.as_deref().unwrap_or("");
            match crw_extract::structured::extract_structured(md, schema, llm).await {
                Ok(json) => data.json = Some(json),
                Err(e) => {
                    tracing::error!("Structured extraction failed: {e}");
                    return Err(e);
                }
            }
        } else if req.json_schema.is_some() && effective_llm.is_none() {
            return Err(crw_core::error::CrwError::ExtractionError(
                "JSON extraction requested but no LLM configured. Either set [extraction.llm] in server config, or pass 'llmApiKey' in the request body.".into()
            ));
        } else if req.json_schema.is_none() {
            return Err(crw_core::error::CrwError::InvalidRequest(
                "Format 'json' requires a jsonSchema field. Provide a JSON Schema object for structured extraction.".into()
            ));
        }
    }

    Ok(data)
}

pub(crate) fn derive_target_warning(fetch_result: &FetchResult) -> Option<String> {
    if fetch_result.warning.is_some() {
        return fetch_result.warning.clone();
    }

    if fetch_result.status_code >= 400 {
        return Some(format!(
            "Target returned {} {}",
            fetch_result.status_code,
            canonical_status_text(fetch_result.status_code)
        ));
    }

    detect_block_interstitial(&fetch_result.html)
}

fn canonical_status_text(status_code: u16) -> &'static str {
    match status_code {
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        410 => "Gone",
        429 => "Too Many Requests",
        451 => "Unavailable For Legal Reasons",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Error",
    }
}

fn detect_block_interstitial(html: &str) -> Option<String> {
    // If page has substantial content (>50KB), it's not a block/interstitial page
    if html.len() > 50_000 {
        return None;
    }

    const SCAN_LIMIT: usize = 128 * 1024;
    let end = if html.len() <= SCAN_LIMIT {
        html.len()
    } else {
        let mut e = SCAN_LIMIT;
        while e > 0 && !html.is_char_boundary(e) {
            e -= 1;
        }
        e
    };
    let lower = html[..end].to_lowercase();
    let markers = [
        "just a moment",
        "attention required",
        "cf-browser-verification",
        "cf-challenge",
        "captcha",
        "access denied",
    ];

    if markers.iter().any(|marker| lower.contains(marker)) {
        Some("Blocked by anti-bot protection".to_string())
    } else {
        None
    }
}

fn formats_include_json(formats: &[OutputFormat]) -> bool {
    formats.contains(&OutputFormat::Json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_fetch(status_code: u16, html: &str) -> FetchResult {
        FetchResult {
            url: "https://example.com".into(),
            status_code,
            html: html.into(),
            rendered_with: None,
            elapsed_ms: 10,
            warning: None,
        }
    }

    #[test]
    fn warning_detects_target_status_codes() {
        let warning = derive_target_warning(&sample_fetch(403, "<html></html>"));
        assert_eq!(warning.as_deref(), Some("Target returned 403 Forbidden"));
    }

    #[test]
    fn warning_detects_block_markers() {
        let warning = derive_target_warning(&sample_fetch(
            200,
            "<html><title>Just a moment</title><body>cf-browser-verification</body></html>",
        ));
        assert_eq!(warning.as_deref(), Some("Blocked by anti-bot protection"));
    }
}
