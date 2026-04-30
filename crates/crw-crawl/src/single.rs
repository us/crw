use crw_core::config::{BUILTIN_UA_POOL, LlmConfig};
use crw_core::error::CrwResult;
use crw_core::types::{
    FetchResult, OutputFormat, ScrapeData, ScrapeRequest, resolve_pinned_renderer,
    resolve_render_js,
};
use crw_renderer::FallbackRenderer;
use crw_renderer::http_only::HttpFetcher;
use crw_renderer::traits::PageFetcher;
use std::sync::Arc;

/// Scrape a single URL: fetch → extract → (optional) LLM structured extraction.
///
/// - `user_agent`: base user-agent string from global config.
/// - `default_stealth`: whether stealth headers are active by global config.
/// - `render_js_default`: global `[renderer] render_js_default` config; used only
///   for the `needs_temp_fetcher` HTTP-only gating. The shared renderer applies
///   the same default internally, so we don't forward it to the renderer call.
pub async fn scrape_url(
    req: &ScrapeRequest,
    renderer: &Arc<FallbackRenderer>,
    llm_config: Option<&LlmConfig>,
    user_agent: &str,
    default_stealth: bool,
    render_js_default: Option<bool>,
) -> CrwResult<ScrapeData> {
    // Reject unsupported `actions` parameter early with a clear error.
    if req.actions.is_some() {
        return Err(crw_core::error::CrwError::InvalidRequest(
            "The 'actions' parameter is not yet supported. Use cssSelector or xpath for element targeting.".into()
        ));
    }

    // Determine whether stealth headers should be injected for this request.
    let inject_stealth = req.stealth.unwrap_or(default_stealth);

    let pinned = resolve_pinned_renderer(req.renderer);

    // "Pinned implies JS" — if user named a non-auto renderer but didn't set
    // renderJs, force JS so auto-gating doesn't silently bypass the pin.
    let effective_render_js_request = if pinned.is_some() && req.render_js.is_none() {
        Some(true)
    } else {
        req.render_js
    };

    // Resolve the effective render_js decision (per-request overrides global default).
    // Used for the temp-fetcher HTTP-only gate below so a user with
    // render_js_default=true and a per-request proxy still reaches the JS renderer.
    let effective_render_js = resolve_render_js(effective_render_js_request, render_js_default);

    // Validate pinned renderer is available — fail fast with a 400 instead of
    // letting the request reach the dispatcher with a hard-pin to a missing pool.
    // Skip validation when renderJs:false is honored (HTTP-only ignores the pin).
    if let Some(name) = pinned
        && effective_render_js != Some(false)
    {
        let available = renderer.js_renderer_names();
        if !available.contains(&name) {
            return Err(crw_core::error::CrwError::InvalidRequest(format!(
                "renderer '{}' not available; configured renderers: [{}]. \
                 Update server config or omit the 'renderer' field.",
                name,
                available.join(", ")
            )));
        }
    }

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
            BUILTIN_UA_POOL[rand::random_range(0..BUILTIN_UA_POOL.len())].to_string()
        } else {
            user_agent.to_string()
        };

        if effective_render_js == Some(false) {
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
                .fetch(
                    &req.url,
                    &merged_headers,
                    effective_render_js_request,
                    req.wait_for,
                    pinned,
                )
                .await?
        }
    } else {
        renderer
            .fetch(
                &req.url,
                &req.headers,
                effective_render_js_request,
                req.wait_for,
                pinned,
            )
            .await?
    };

    // PDF routing: skip HTML extraction pipeline
    #[cfg(feature = "pdf")]
    if fetch_result.content_type.as_deref() == Some("application/pdf")
        && let Some(bytes) = &fetch_result.raw_bytes
    {
        let mut data = crw_extract::pdf::extract_pdf(
            bytes,
            &fetch_result.url,
            fetch_result.status_code,
            fetch_result.elapsed_ms,
            &req.formats,
        )?;

        // LLM structured extraction for PDF content
        let effective_schema = req
            .json_schema
            .as_ref()
            .or_else(|| req.extract.as_ref().and_then(|e| e.schema.as_ref()));

        if formats_include_json(&req.formats) {
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

            if let (Some(schema), Some(llm)) = (effective_schema, effective_llm) {
                let md = data.markdown.as_deref().unwrap_or("");
                match crw_extract::structured::extract_structured(md, schema, llm).await {
                    Ok(json) => data.json = Some(json),
                    Err(e) => {
                        tracing::error!("PDF structured extraction failed: {e}");
                        return Err(e);
                    }
                }
            } else if effective_schema.is_some() && effective_llm.is_none() {
                return Err(crw_core::error::CrwError::ExtractionError(
                    "JSON extraction requested but no LLM configured. Either set [extraction.llm] in server config, or pass 'llmApiKey' in the request body.".into()
                ));
            } else if effective_schema.is_none() {
                return Err(crw_core::error::CrwError::InvalidRequest(
                    "Structured extraction (formats: json/extract) requires a 'jsonSchema' field. Provide a JSON Schema object.".into()
                ));
            }
        }

        return Ok(data);
    }

    let warning = derive_target_warning(&fetch_result);
    fn build_extract_opts<'a>(
        fr: &'a FetchResult,
        req: &'a ScrapeRequest,
    ) -> crw_extract::ExtractOptions<'a> {
        crw_extract::ExtractOptions {
            raw_html: &fr.html,
            source_url: &fr.url,
            status_code: fr.status_code,
            rendered_with: fr.rendered_with.clone(),
            elapsed_ms: fr.elapsed_ms,
            render_decision: fr.render_decision.clone(),
            credit_cost: fr.credit_cost,
            warnings: fr.warnings.clone(),
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
        }
    }
    let mut data = crw_extract::extract(build_extract_opts(&fetch_result, req))?;

    // Post-extract escalation: HTTP-only fetch returned 2xx but extraction
    // produced no markdown. Re-fetch with JS rendering forced. Catches sites
    // whose HTML is substantive (so `looks_like_thin_html` doesn't trigger at
    // the renderer layer) but whose content lives entirely in client-side
    // hydration or post-load shadow DOM. Bench analysis: ~13/147 failures.
    let md_is_empty = data
        .markdown
        .as_deref()
        .map(|s| s.trim().len() < 100)
        .unwrap_or(true);
    let used_http_only = matches!(
        fetch_result.rendered_with.as_deref(),
        Some("http") | Some("http_only_fallback")
    );
    let is_2xx = (200..300).contains(&fetch_result.status_code);
    let escalation_eligible = effective_render_js != Some(false)
        && !needs_temp_fetcher
        && !renderer.js_renderer_names().is_empty()
        && req.formats.contains(&OutputFormat::Markdown);

    if md_is_empty && used_http_only && is_2xx && escalation_eligible {
        tracing::info!(
            url = %req.url,
            html_len = fetch_result.html.len(),
            "HTTP 2xx but markdown extraction empty, escalating to JS renderer"
        );
        match renderer
            .fetch(&req.url, &req.headers, Some(true), req.wait_for, pinned)
            .await
        {
            Ok(js_fetch) if js_fetch.status_code < 400 => {
                if let Ok(js_data) = crw_extract::extract(build_extract_opts(&js_fetch, req)) {
                    let js_md_len = js_data
                        .markdown
                        .as_deref()
                        .map(|s| s.trim().len())
                        .unwrap_or(0);
                    if js_md_len >= 100 {
                        data = js_data;
                    }
                }
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(url = %req.url, "JS escalation after empty markdown failed: {e}");
            }
        }
    }
    // Merge target warning with any extraction warning (e.g. orphan chunk params).
    data.warning = match (warning, data.warning) {
        (Some(w1), Some(w2)) => Some(format!("{w1}; {w2}")),
        (Some(w), None) | (None, Some(w)) => Some(w),
        (None, None) => None,
    };

    // Phase 4: LLM structured extraction
    // Merge Firecrawl-compatible extract.schema into json_schema if not already set.
    let effective_schema = req
        .json_schema
        .as_ref()
        .or_else(|| req.extract.as_ref().and_then(|e| e.schema.as_ref()));

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

        if let (Some(schema), Some(llm)) = (effective_schema, effective_llm) {
            let md = data.markdown.as_deref().unwrap_or("");
            match crw_extract::structured::extract_structured(md, schema, llm).await {
                Ok(json) => data.json = Some(json),
                Err(e) => {
                    tracing::error!("Structured extraction failed: {e}");
                    return Err(e);
                }
            }
        } else if effective_schema.is_some() && effective_llm.is_none() {
            return Err(crw_core::error::CrwError::ExtractionError(
                "JSON extraction requested but no LLM configured. Either set [extraction.llm] in server config, or pass 'llmApiKey' in the request body.".into()
            ));
        } else if effective_schema.is_none() {
            return Err(crw_core::error::CrwError::InvalidRequest(
                "Structured extraction (formats: json/extract) requires a 'jsonSchema' field. Provide a JSON Schema object.".into()
            ));
        }
    }

    Ok(data)
}

pub(crate) fn derive_target_warning(fetch_result: &FetchResult) -> Option<String> {
    // Anti-bot detection wins over any other warning. The renderer chain
    // annotates thin results with "X returned a loading placeholder", but the
    // underlying HTML may be a CAPTCHA shell — surfacing the placeholder
    // misattributes the failure to our renderer instead of the site block.
    if let Some(block) = detect_block_interstitial(&fetch_result.html) {
        return Some(block);
    }

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

    None
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
        // DataDome (Reuters, Forbes, Inc, Zoro, WSJ, etc.) — serves a small
        // CAPTCHA/iframe shell to headless browsers. The captcha-delivery host
        // and "datadome" string only appear on actively-challenged pages.
        "captcha-delivery.com",
        "datadome captcha",
        "data-cfasync",
        // PerimeterX / HUMAN
        "px-captcha",
        "_px3=",
        // Akamai Bot Manager
        "_abck=",
        "ak-challenge",
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
            content_type: None,
            raw_bytes: None,
            rendered_with: None,
            elapsed_ms: 10,
            warning: None,
            render_decision: None,
            credit_cost: 0,
            warnings: Vec::new(),
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
