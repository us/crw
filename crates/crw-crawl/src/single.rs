use crw_core::Deadline;
use crw_core::config::{BUILTIN_UA_POOL, ExtractionConfig, LlmConfig};
use crw_core::error::CrwResult;
use crw_core::types::{
    ChangeTrackingMode, FetchResult, OutputFormat, ScrapeData, ScrapeRequest,
    resolve_pinned_renderer, resolve_render_js,
};
use crw_renderer::FallbackRenderer;
use crw_renderer::http_only::HttpFetcher;
use crw_renderer::traits::PageFetcher;
use std::sync::{Arc, Mutex};

/// Resolve the effective proxy for a request, honoring precedence
/// `req.proxy_list > req.proxy > server config`. The single resolved entry is
/// scoped into `REQUEST_PROXY` so BOTH the HTTP and JS/CDP paths egress through
/// it (no second pick, no path-specific resolution). A malformed BYOP proxy is
/// an `InvalidRequest` error — never a silent direct connection.
fn resolve_request_proxy(
    req: &ScrapeRequest,
    renderer: &FallbackRenderer,
) -> CrwResult<Option<Arc<crw_core::ProxyEntry>>> {
    if !req.proxy_list.is_empty() || req.proxy.is_some() {
        let byop = crw_core::ProxyRotator::build(
            &req.proxy_list,
            req.proxy.as_deref(),
            req.proxy_rotation.unwrap_or_default(),
        )
        .map_err(crw_core::error::CrwError::InvalidRequest)?;
        if let Some(byop) = byop {
            let host = url::Url::parse(&req.url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_string));
            return Ok(Some(Arc::new(byop.pick(host.as_deref()).clone())));
        }
    }
    Ok(renderer.pick_proxy_for_url(&req.url))
}

/// Scrape a single URL: fetch → extract → (optional) LLM structured extraction.
///
/// - `user_agent`: base user-agent string from global config.
/// - `default_stealth`: whether stealth headers are active by global config.
/// - `render_js_default`: global `[renderer] render_js_default` config; used only
///   for the `needs_temp_fetcher` HTTP-only gating. The shared renderer applies
///   the same default internally, so we don't forward it to the renderer call.
#[allow(clippy::too_many_arguments)]
pub async fn scrape_url(
    req: &ScrapeRequest,
    renderer: &Arc<FallbackRenderer>,
    llm_config: Option<&LlmConfig>,
    extraction_cfg: &ExtractionConfig,
    user_agent: &str,
    default_stealth: bool,
    render_js_default: Option<bool>,
    deadline: Deadline,
) -> CrwResult<ScrapeData> {
    // Propagate per-request country + the single resolved proxy into the
    // renderer stack via task-locals. `REQUEST_COUNTRY` drives DataImpulse
    // credential country; `REQUEST_PROXY` carries the resolved proxy (BYOP >
    // config) so BOTH the HTTP and JS/CDP paths egress through the same entry.
    let resolved_proxy = resolve_request_proxy(req, renderer)?;
    // `REQUEST_SCREENSHOT` carries the (out-of-band) screenshot params into the
    // renderer stack so the CDP path can capture without trait-signature churn
    // (mirrors REQUEST_PROXY). `Some` only when `formats` asked for it.
    let screenshot_req =
        req.formats
            .contains(&OutputFormat::Screenshot)
            .then_some(crw_renderer::ScreenshotReq {
                full_page: req.screenshot_full_page,
            });
    crw_renderer::REQUEST_COUNTRY
        .scope(req.country.clone(), async move {
            crw_renderer::REQUEST_PROXY
                .scope(resolved_proxy, async move {
                    crw_renderer::REQUEST_SCREENSHOT
                        .scope(screenshot_req, async move {
                            scrape_url_inner(
                                req,
                                renderer,
                                llm_config,
                                extraction_cfg,
                                user_agent,
                                default_stealth,
                                render_js_default,
                                deadline,
                            )
                            .await
                        })
                        .await
                })
                .await
        })
        .await
}

#[allow(clippy::too_many_arguments)]
async fn scrape_url_inner(
    req: &ScrapeRequest,
    renderer: &Arc<FallbackRenderer>,
    llm_config: Option<&LlmConfig>,
    extraction_cfg: &ExtractionConfig,
    user_agent: &str,
    default_stealth: bool,
    render_js_default: Option<bool>,
    deadline: Deadline,
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

    // A screenshot is captured via CDP and cannot be produced on the HTTP-only
    // path. An explicit `renderJs:false` + `screenshot` is contradictory — reject
    // it rather than silently return a null screenshot. For the default/auto case
    // the renderer forces the CDP path (see FallbackRenderer::fetch), and the
    // temp HTTP fetcher below is skipped so the screenshot is never dropped.
    let wants_screenshot = req.formats.contains(&OutputFormat::Screenshot);
    if wants_screenshot && req.render_js == Some(false) {
        return Err(crw_core::error::CrwError::InvalidRequest(
            "screenshot format requires JS rendering; remove renderJs:false (or omit it)".into(),
        ));
    }

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

    // Use a temporary fetcher ONLY when per-request stealth differs from the
    // shared renderer's config. Proxy egress (config rotation + BYOP) is carried
    // by the REQUEST_PROXY task-local (resolved once in `scrape_url`) and is
    // honored by the shared renderer's HTTP and CDP paths, so it does not need a
    // temp fetcher.
    let needs_temp_fetcher = req.stealth.is_some_and(|s| s != default_stealth);

    let mut fetch_result = if needs_temp_fetcher {
        // Rotate UA from built-in pool when stealth is active, so the request
        // looks like a real browser even for per-request stealth overrides.
        let effective_ua = if inject_stealth {
            BUILTIN_UA_POOL[rand::random_range(0..BUILTIN_UA_POOL.len())].to_string()
        } else {
            user_agent.to_string()
        };

        if effective_render_js == Some(false) && !wants_screenshot {
            // HTTP-only temp fetcher with per-request stealth. Honor REQUEST_PROXY
            // so a stealth-override request still egresses through the resolved
            // proxy — fail-closed, a set proxy is never bypassed.
            let temp_http = match crw_renderer::REQUEST_PROXY
                .try_with(|p| p.clone())
                .ok()
                .flatten()
            {
                Some(entry) => HttpFetcher::with_proxy(
                    &effective_ua,
                    entry.raw(),
                    inject_stealth,
                    std::time::Duration::from_secs(30),
                )?,
                None => HttpFetcher::new(&effective_ua, None, inject_stealth),
            };
            temp_http
                .fetch(&req.url, &req.headers, req.wait_for, deadline)
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
                    deadline,
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
                deadline,
            )
            .await?
    };

    let warning = derive_target_warning(&fetch_result);
    // Per-request debug collector — shared across the multi-attempt JS
    // escalation so all candidate ladders land in one trace.
    let debug_enabled = req.debug.unwrap_or(false);
    let debug_sink: Option<Arc<Mutex<crw_extract::DebugCollector>>> = if debug_enabled {
        Some(Arc::new(Mutex::new(crw_extract::DebugCollector::new())))
    } else {
        None
    };
    // Build the OWNED extraction input so the CPU-bound `extract()` can run off
    // the async reactor via `extract_pool::extract_offloaded` (spawn_blocking
    // needs `'static`, so the borrowed `ExtractOptions` can't cross the
    // boundary). `domain_selectors` is wrapped in an `Arc` to avoid deep-copying
    // the host→selector map on every request.
    fn build_owned_extract_input(
        fr: &FetchResult,
        req: &ScrapeRequest,
        extraction_cfg: &ExtractionConfig,
        debug: bool,
        sink: Option<Arc<Mutex<crw_extract::DebugCollector>>>,
    ) -> crw_extract::OwnedExtractInput {
        crw_extract::OwnedExtractInput {
            raw_html: fr.html.clone(),
            source_url: fr.url.clone(),
            status_code: fr.status_code,
            rendered_with: fr.rendered_with.clone(),
            elapsed_ms: fr.elapsed_ms,
            render_decision: fr.render_decision.clone(),
            credit_cost: fr.credit_cost,
            warnings: fr.warnings.clone(),
            formats: req.formats.clone(),
            only_main_content: req.only_main_content,
            include_tags: req.include_tags.clone(),
            exclude_tags: req.exclude_tags.clone(),
            css_selector: req.css_selector.clone(),
            xpath: req.xpath.clone(),
            chunk_strategy: req.chunk_strategy.clone(),
            query: req.query.clone(),
            filter_mode: req.filter_mode.clone(),
            top_k: req.top_k,
            domain_selectors: Some(Arc::new(extraction_cfg.domain_selectors.clone())),
            captured_responses: fr.captured_responses.clone(),
            debug,
            debug_sink: sink,
        }
    }
    // ── PDF document branch ────────────────────────────────────────────────
    // When the HTTP renderer captured a PDF body (`raw_bytes`), convert it to
    // markdown via pdf-inspector instead of running the HTML pipeline. Sits
    // BEFORE extract() so every `scrape_url` caller (single scrape, crawl item,
    // search enrichment, batch) inherits PDF support for free. The HTML
    // cleaning + JS-escalation paths are skipped entirely for PDFs; the shared
    // downstream stages (LLM json/summary, change-tracking) run unchanged on
    // the produced `data.markdown` / `data.content_type`.
    let pdf_bytes = if fetch_result.content_type.as_deref() == Some("application/pdf")
        && crate::pdf::pdf_parse_requested(req)
    {
        fetch_result.raw_bytes.take()
    } else {
        None
    };

    let mut effective_warning = warning;
    let mut data = if let Some(bytes) = pdf_bytes {
        let source = crate::pdf::PdfSource {
            source_url: fetch_result.url.clone(),
            status_code: fetch_result.status_code,
            elapsed_ms: fetch_result.elapsed_ms,
            source_filename: None,
        };
        crate::pdf::convert_pdf_bytes(bytes, req, source).await?
    } else {
        let mut data = crate::extract_pool::extract_offloaded(build_owned_extract_input(
            &fetch_result,
            req,
            extraction_cfg,
            debug_enabled,
            debug_sink.clone(),
        ))
        .await?;
        // LLM-assisted re-extraction when DOM result is low-quality and the
        // operator opted in via [extraction.llm_fallback]. Failure paths inside
        // the helper preserve the original markdown.
        if extraction_cfg.llm_fallback.enable
            && let Some(llm_cfg) = llm_config.or(extraction_cfg.llm.as_ref())
        {
            let params = crw_extract::LlmFallbackParams {
                api_key: &llm_cfg.api_key,
                model: &llm_cfg.model,
                provider: &llm_cfg.provider,
                base_url: llm_cfg.base_url.as_deref(),
                quality_threshold: extraction_cfg.llm_fallback.quality_threshold,
                max_html_bytes: extraction_cfg.llm_fallback.max_html_bytes,
                max_tokens: llm_cfg.max_tokens,
                azure_api_version: llm_cfg.azure_api_version.as_deref(),
                always_run: extraction_cfg.llm_fallback.always_run,
            };
            let _ =
                crw_extract::maybe_run_llm_fallback(&mut data, &fetch_result.html, &params).await;
        }

        // Post-extract escalation: HTTP-only fetch returned 2xx but extraction
        // produced no markdown. Re-fetch with JS rendering forced. Catches sites
        // whose HTML is substantive (so `looks_like_thin_html` doesn't trigger at
        // the renderer layer) but whose content lives entirely in client-side
        // hydration or post-load shadow DOM. Bench analysis: ~13/147 failures.
        // Threshold for "empty enough to trigger an escalation".
        //   - HTTP tier: 100 bytes is enough — even a basic shell exceeds that.
        //   - LightPanda tier: 500 bytes. LightPanda routinely returns 90–200 byte
        //     SPA husks (just <head> + a hydration sentinel) that pass the 100-byte
        //     bar but contain nothing the user wants. Bench analysis showed 6 URLs
        //     where chrome retrieves the full page after lightpanda gave us a 90B
        //     stub (bandbhdwr, cascadehomecenter, laportehardware, apploi,
        //     indiamart, zujuan.xkw) — bumping the lightpanda-only threshold to
        //     500 captures all of them without changing http-tier behavior.
        // Tier of renderer that produced fetch_result. We always escalate from
        // "below" — http and lightpanda → try chrome — but never re-call chrome
        // when chrome already produced the empty markdown (that would just churn).
        // Thresholds default to 100B (http) and 2000B (lightpanda); both are
        // overridable via [extraction] in server config so operators can tune
        // per-deployment without recompiling.
        let prior_renderer = fetch_result.rendered_with.as_deref();
        let retry_threshold = if prior_renderer == Some("lightpanda") {
            extraction_cfg.lightpanda_retry_threshold_bytes
        } else {
            extraction_cfg.http_retry_threshold_bytes
        };
        let md_bytes = data
            .markdown
            .as_deref()
            .map(|s| s.trim().len())
            .unwrap_or(0);
        let md_is_byte_thin = md_bytes < retry_threshold;
        let md_quality = data
            .markdown
            .as_deref()
            .map(crw_extract::quality::analyze_md_only);
        let md_is_low_quality = md_quality
            .as_ref()
            .is_some_and(crw_extract::quality::is_low_quality);
        let used_low_tier = matches!(
            prior_renderer,
            Some("http") | Some("http_only_fallback") | Some("lightpanda")
        );
        // Only escalate on 2xx here. Renderer-level (lib.rs) already handles
        // soft-block status codes (401/403/405/406/410/412/429/451/503) via its
        // own `is_auth_blocked` path; running another escalation from this layer
        // would just hit the same circuit breakers a second time and waste a
        // request budget. Our job here is the 2xx-with-empty-markdown gap that
        // the renderer's HTML-shape thinness heuristic doesn't catch.
        let should_escalate_status = (200..300).contains(&fetch_result.status_code);
        let escalation_eligible = effective_render_js != Some(false)
        && !needs_temp_fetcher
        && !renderer.js_renderer_names().is_empty()
        && req.formats.contains(&OutputFormat::Markdown)
        // Never JS-render a PDF: even when parsing is disabled (`parsers: []`)
        // the document has no DOM to escalate into.
        && fetch_result.content_type.as_deref() != Some("application/pdf");

        let escalate_for_quality =
            !md_is_byte_thin && md_is_low_quality && fetch_result.html.len() > 5000;
        let should_escalate = (md_is_byte_thin || escalate_for_quality)
            && used_low_tier
            && should_escalate_status
            && escalation_eligible;
        if should_escalate {
            // If the prior tier was lightpanda (returned 200 with thin/no content
            // that fooled the renderer-level thinness check), force chrome on the
            // escalation. Falling back to "auto" would just hit lightpanda again.
            // Otherwise (http tier), let the chain decide so chrome can be reached
            // through the existing failover path.
            let escalation_target: Option<&str> = if prior_renderer == Some("lightpanda") {
                Some("chrome")
            } else {
                pinned
            };
            let quality_score_before = md_quality.as_ref().map(|q| q.score);
            tracing::info!(
                url = %req.url,
                status = fetch_result.status_code,
                html_len = fetch_result.html.len(),
                prior = prior_renderer,
                target = escalation_target,
                md_bytes,
                quality_score_before = ?quality_score_before,
                escalate_for_quality,
                "empty markdown after fetch, escalating to JS renderer"
            );
            match renderer
                .fetch(
                    &req.url,
                    &req.headers,
                    Some(true),
                    req.wait_for,
                    escalation_target,
                    deadline,
                )
                .await
            {
                Ok(mut js_fetch) => {
                    // Accept JS result even if status >= 400, as long as it produced
                    // real content. Anti-bot/UA-detection sites frequently return a
                    // 4xx code while still serving the actual page body — the status
                    // is a soft signal, not a content gate.
                    let js_status = js_fetch.status_code;
                    let js_warning = derive_target_warning(&js_fetch);
                    if let Ok(js_data) =
                        crate::extract_pool::extract_offloaded(build_owned_extract_input(
                            &js_fetch,
                            req,
                            extraction_cfg,
                            debug_enabled,
                            debug_sink.clone(),
                        ))
                        .await
                    {
                        let js_md_len = js_data
                            .markdown
                            .as_deref()
                            .map(|s| s.trim().len())
                            .unwrap_or(0);
                        let js_md_quality = js_data
                            .markdown
                            .as_deref()
                            .map(crw_extract::quality::analyze_md_only);
                        let js_score = js_md_quality.as_ref().map(|q| q.score).unwrap_or(0.0);
                        let before_score = md_quality.as_ref().map(|q| q.score).unwrap_or(0.0);
                        let http_was_thin = md_is_byte_thin;
                        let quality_improved = js_score > before_score + 0.05;
                        let accept =
                            js_md_len >= retry_threshold && (http_was_thin || quality_improved);
                        if accept {
                            data = js_data;
                            // The escalation re-rendered via CDP, so a screenshot (if
                            // requested) lives on `js_fetch`, not the original low-tier
                            // `fetch_result`. Carry it over so it isn't dropped.
                            fetch_result.screenshot = js_fetch.screenshot.take();
                            // Replace the original "Target returned 4xx" with the JS
                            // fetch's warning (which is None for a clean 2xx render),
                            // so a successful escalation doesn't leak the original
                            // soft-block status into the response top-level warning.
                            effective_warning = js_warning;
                            tracing::info!(
                                url = %req.url,
                                from_status = fetch_result.status_code,
                                to_status = js_status,
                                md_len = js_md_len,
                                quality_score_before = before_score,
                                quality_score_after = js_score,
                                "JS escalation recovered content"
                            );
                        } else if js_md_len >= retry_threshold && !http_was_thin {
                            tracing::info!(
                                url = %req.url,
                                before = before_score,
                                after = js_score,
                                "JS retry returned worse-quality markdown ({before_score} -> {js_score}), keeping HTTP",
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(url = %req.url, "JS escalation after empty markdown failed: {e}");
                }
            }
        }
        data
    };
    // Surface redirect mismatch as warning. Helps detect cases like
    // northernair.ca/history.htm silently 302'ing to the homepage — extraction
    // looks "successful" but the user got the wrong page.
    if let Some(final_url) = fetch_result.final_url.as_deref()
        && redirect_is_material(&fetch_result.url, final_url)
    {
        let warning = format!("redirected_to: {final_url}");
        if !data.warnings.iter().any(|w| w == &warning) {
            data.warnings.push(warning);
        }
    }

    // Merge target warning with any extraction warning (e.g. orphan chunk params).
    data.warning = match (effective_warning, data.warning) {
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

    // Build BYOK LlmConfig once; reused by structured JSON + summary paths.
    let byok_config = build_byok_llm_config(req, llm_config);
    let effective_llm = byok_config.as_ref().or(llm_config);

    if formats_include_json(&req.formats) {
        if let (Some(schema), Some(llm)) = (effective_schema, effective_llm) {
            let md = data.markdown.as_deref().unwrap_or("");
            match crw_extract::structured::extract_structured_with_usage(md, schema, llm, None)
                .await
            {
                Ok(result) => {
                    data.json = Some(result.value);
                    // Surface per-call LLM token usage so callers (billing,
                    // dashboards) see the structured-extraction spend.
                    // Summary may overwrite this slot below; that's fine —
                    // each route triggers at most one of the two paths in
                    // the dominant flow, and the "first wins" tiebreak is
                    // preserved by checking is_none() before assignment.
                    if data.llm_usage.is_none() {
                        data.llm_usage = result.usage;
                    }
                }
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

    if formats_include_summary(&req.formats) {
        let Some(llm) = effective_llm else {
            return Err(crw_core::error::CrwError::ExtractionError(
                "Summary format requires an LLM config. Either set [extraction.llm] in server config, or pass 'llmApiKey' in the request body.".into()
            ));
        };
        // Markdown is computed internally even if not in `formats`; if the
        // caller asked only for `summary`, the markdown is the input to the
        // LLM but is not surfaced in the response (see strip below).
        let md_owned = data.markdown.clone().unwrap_or_default();
        match crw_extract::summary::summarize(
            &md_owned,
            llm,
            req.summary_prompt.as_deref(),
            req.max_content_chars,
        )
        .await
        {
            Ok(result) => {
                data.summary = Some(result.content);
                if data.llm_usage.is_none() {
                    data.llm_usage = result.usage;
                }
                if let Some(w) = result.warning {
                    data.warnings.push(w);
                }
            }
            Err(e) => {
                tracing::warn!("Summary generation failed: {e}");
                data.warnings.push(format!("summary failed: {e}"));
            }
        }
        // If the caller didn't explicitly ask for markdown, strip the
        // internally-computed markdown from the response.
        if !req.formats.contains(&OutputFormat::Markdown) {
            data.markdown = None;
        }
    }

    // Drain the per-request debug sink into ScrapeData. The sink is the
    // last shared owner at this point — extract() returned, dropping its
    // clone — so try_unwrap should succeed; if a stray clone is alive we
    // fall back to a clone of the inner Vec.
    if let Some(sink) = debug_sink {
        // Each extract() call dropped its clone of the Arc, so by this
        // point we hold the only reference and can unwrap cheaply.
        let extraction = match Arc::try_unwrap(sink) {
            Ok(mu) => mu.into_inner().unwrap_or_default().into_extraction(),
            Err(_) => crw_core::types::DebugExtraction::default(),
        };
        data.debug_extraction = Some(extraction);
    }

    // Surface the fetched content type so change-tracking (here and on the
    // crawl path) can hash binary/non-text content rather than diff it.
    data.content_type = fetch_result.content_type.clone();

    // Wrap the raw base64 screenshot in a `data:image/png;base64,` URL exactly
    // once, here, so both v1 and v2 responses are identical (D8). FetchResult
    // keeps the raw b64.
    data.screenshot = fetch_result
        .screenshot
        .as_ref()
        .map(|b| format!("data:image/png;base64,{b}"));

    // ── Change tracking (monitor) ──────────────────────────────────────────
    // Activated by the `"changeTracking"` format string; options ride on the
    // sibling `change_tracking` field. The diff is computed against the
    // caller-supplied `previous` snapshot — opencore stores nothing. The LLM
    // judge is injected by the M2 orchestration layer, not here.
    if req.formats.contains(&OutputFormat::ChangeTracking) {
        let Some(ct_opts) = &req.change_tracking else {
            return Err(crw_core::error::CrwError::InvalidRequest(
                "formats includes 'changeTracking' but no 'changeTracking' options were provided."
                    .into(),
            ));
        };
        let wants_json = ct_opts.modes.contains(&ChangeTrackingMode::Json);

        // For json / mixed mode, extract the tracked fields using the
        // changeTracking schema (independent of the top-level `json` format).
        let mut current_json: Option<serde_json::Value> = None;
        if wants_json {
            match (ct_opts.schema.as_ref(), effective_llm) {
                (Some(schema), Some(llm)) => {
                    let md = data.markdown.as_deref().unwrap_or("");
                    match crw_extract::structured::extract_structured_with_usage(
                        md, schema, llm, None,
                    )
                    .await
                    {
                        Ok(result) => {
                            current_json = Some(result.value);
                            if data.llm_usage.is_none() {
                                data.llm_usage = result.usage;
                            }
                        }
                        Err(e) => return Err(e),
                    }
                }
                (None, _) => {
                    return Err(crw_core::error::CrwError::InvalidRequest(
                        "changeTracking json mode requires a 'schema' describing the fields to track.".into(),
                    ));
                }
                (Some(_), None) => {
                    return Err(crw_core::error::CrwError::ExtractionError(
                        "changeTracking json mode requires an LLM config. Set [extraction.llm] or pass 'llmApiKey'.".into(),
                    ));
                }
            }
        }

        let md = data.markdown.as_deref().unwrap_or("");
        let started = std::time::Instant::now();
        let mut result = crw_diff::compute_change_tracking(
            ct_opts,
            md,
            current_json.as_ref(),
            data.content_type.as_deref(),
        );

        // Observability: diff duration + retained snapshot size, by mode.
        let mode = change_tracking_mode_label(ct_opts, data.content_type.as_deref());
        let m = crw_core::metrics::metrics();
        m.change_tracking_duration_seconds
            .with_label_values(&[mode])
            .observe(started.elapsed().as_secs_f64());
        if let Some(snap) = &result.snapshot {
            let bytes = snap.markdown.as_ref().map(|s| s.len()).unwrap_or(0)
                + snap.json.as_ref().map(|j| j.to_string().len()).unwrap_or(0);
            m.change_tracking_snapshot_bytes
                .with_label_values(&[mode])
                .observe(bytes as f64);
        }

        // ── Meaningful-change judge (M2) ──────────────────────────────────
        // Runs only on a changed page that produced a diff (excludes binary
        // and first-observation pages), when a goal is set and judging is
        // enabled. Judge failure never fails the scrape — it degrades to no
        // judgment plus a warning. opencore does no credit math; the SaaS
        // bills a flat +1 credit per judged changed page.
        if result.status == crw_core::types::ChangeStatus::Changed
            && result.diff.is_some()
            && req.judge_enabled == Some(true)
            && let Some(goal) = req.goal.as_deref().map(str::trim).filter(|g| !g.is_empty())
        {
            let has_json = ct_opts.modes.contains(&ChangeTrackingMode::Json);
            let diff_text = result.diff.as_ref().and_then(|d| d.text.as_deref());
            // Only the per-field json map (json/mixed) is a useful judge input;
            // the gitDiff-only AST under diff.json is not field-level changes.
            let json_diff = if has_json {
                result.diff.as_ref().and_then(|d| d.json.as_ref())
            } else {
                None
            };
            match effective_llm {
                Some(llm) => {
                    match crw_extract::judge::judge_change(goal, diff_text, json_diff, llm, None)
                        .await
                    {
                        Ok(judgment) => {
                            m.judge_calls_total.with_label_values(&["ok"]).inc();
                            if let Some(u) = &judgment.llm_usage {
                                m.judge_tokens_total
                                    .with_label_values(&["input"])
                                    .inc_by(u.input_tokens as u64);
                                m.judge_tokens_total
                                    .with_label_values(&["output"])
                                    .inc_by(u.output_tokens as u64);
                            }
                            result.judgment = Some(judgment);
                        }
                        Err(e) => {
                            m.judge_calls_total.with_label_values(&["error"]).inc();
                            tracing::warn!("change-tracking judge failed: {e}");
                            data.warnings.push(format!("judge failed: {e}"));
                        }
                    }
                }
                None => {
                    m.judge_calls_total.with_label_values(&["skipped"]).inc();
                    data.warnings
                        .push("judge skipped: no LLM configured".into());
                }
            }
        }

        data.change_tracking = Some(result);
    }

    Ok(data)
}

/// Metric label for a change-tracking computation: `binary` when the content
/// type is non-text, else `mixed` / `json` / `gitDiff` per the active modes.
fn change_tracking_mode_label(
    opts: &crw_core::types::ChangeTrackingOptions,
    content_type: Option<&str>,
) -> &'static str {
    let is_text = content_type.is_none_or(|ct| {
        let ct = ct.to_ascii_lowercase();
        ct.starts_with("text/")
            || ct.contains("json")
            || ct.contains("xml")
            || ct.contains("html")
            || ct.contains("markdown")
            || ct.contains("javascript")
            || ct.contains("csv")
            || ct.contains("yaml")
    });
    if !is_text {
        return "binary";
    }
    let has_git = opts.modes.is_empty() || opts.modes.contains(&ChangeTrackingMode::GitDiff);
    let has_json = opts.modes.contains(&ChangeTrackingMode::Json);
    match (has_git, has_json) {
        (true, true) => "mixed",
        (false, true) => "json",
        _ => "gitDiff",
    }
}

/// Decide whether `final_url` represents a material redirect from `requested`.
/// Returns true when the host changed, or when the requested path was a
/// non-root resource (e.g. `/history.htm`) but the final URL collapsed to the
/// site root (`/` or empty). Pure same-origin path tweaks (trailing slash,
/// query string changes) are ignored.
fn redirect_is_material(requested: &str, final_url: &str) -> bool {
    let Ok(req) = url::Url::parse(requested) else {
        return false;
    };
    let Ok(fin) = url::Url::parse(final_url) else {
        return false;
    };
    if req.host_str() != fin.host_str() {
        return true;
    }
    let req_path = req.path().trim_end_matches('/');
    let fin_path = fin.path().trim_end_matches('/');
    !req_path.is_empty() && fin_path.is_empty()
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
    // Keep markers SPECIFIC to interstitial pages — bare "captcha"/"access
    // denied" false-positive on legit content (e.g. an HN headline mentioning
    // "reCAPTCHA" matches "captcha" anywhere in the document).
    let markers = [
        "just a moment",
        "attention required",
        "cf-browser-verification",
        "cf-challenge",
        // DataDome — captcha-delivery host + "datadome" string only appear on
        // actively-challenged pages.
        "captcha-delivery.com",
        "datadome captcha",
        // PerimeterX / HUMAN — _px3 cookie + px-captcha widget
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

fn formats_include_summary(formats: &[OutputFormat]) -> bool {
    formats.contains(&OutputFormat::Summary)
}

/// Build an `LlmConfig` from per-request BYOK fields, falling back to the
/// server-config values for non-credential fields (concurrency, header
/// guard) so a single request can't escape global limits.
fn build_byok_llm_config(req: &ScrapeRequest, server_cfg: Option<&LlmConfig>) -> Option<LlmConfig> {
    let api_key = req.llm_api_key.as_ref()?.clone();
    let mut cfg = match server_cfg {
        Some(s) => s.clone(),
        None => LlmConfig::default(),
    };
    cfg.api_key = api_key;
    if let Some(p) = &req.llm_provider {
        cfg.provider = p.clone();
    }
    if let Some(m) = &req.llm_model {
        cfg.model = m.clone();
    }
    if let Some(b) = &req.base_url {
        cfg.base_url = Some(b.clone());
    }
    Some(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_fetch(status_code: u16, html: &str) -> FetchResult {
        FetchResult {
            url: "https://example.com".into(),
            final_url: None,
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
            truncated: false,
            deadline_exceeded: false,
            captured_responses: Vec::new(),
            screenshot: None,
        }
    }

    /// The raw base64 in `FetchResult.screenshot` is wrapped into a
    /// `data:image/png;base64,` URL exactly once when building `ScrapeData`.
    #[test]
    fn screenshot_wrapped_as_data_url() {
        let mut fetch = sample_fetch(200, "<html><body>hi</body></html>");
        fetch.screenshot = Some("AAAQ".to_string());
        let wrapped = fetch
            .screenshot
            .as_ref()
            .map(|b| format!("data:image/png;base64,{b}"));
        assert_eq!(
            wrapped.as_deref(),
            Some("data:image/png;base64,AAAQ"),
            "raw b64 must be prefixed with the data URL scheme exactly once"
        );
    }

    #[test]
    fn redirect_material_detects_path_to_root_collapse() {
        assert!(redirect_is_material(
            "https://northernair.ca/history.htm",
            "https://northernair.ca/"
        ));
    }

    #[test]
    fn redirect_material_detects_host_change() {
        assert!(redirect_is_material(
            "https://example.com/path",
            "https://other.com/path"
        ));
    }

    #[test]
    fn redirect_material_ignores_trailing_slash() {
        assert!(!redirect_is_material(
            "https://example.com/path",
            "https://example.com/path/"
        ));
    }

    #[test]
    fn redirect_material_ignores_query_only_change() {
        assert!(!redirect_is_material(
            "https://example.com/page",
            "https://example.com/page?utm=x"
        ));
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

    #[test]
    fn warning_skips_legit_pages_mentioning_captcha() {
        // Regression: HN front page used to false-positive because the headline
        // "Google broke reCAPTCHA…" matched a bare "captcha" substring marker.
        let warning = derive_target_warning(&sample_fetch(
            200,
            "<html><body>Google broke reCAPTCHA for de-googled Android users</body></html>",
        ));
        assert!(warning.is_none(), "got false-positive: {warning:?}");
    }
}
