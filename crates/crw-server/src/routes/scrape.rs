use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use crw_core::Deadline;
use crw_core::error::CrwError;
use crw_core::types::{ApiResponse, OutputFormat, ScrapeData, ScrapeRequest};
use crw_crawl::single::scrape_url;

use crate::error::AppError;
use crate::state::{AppState, validate_renderer_pin};

pub async fn scrape(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    body: Result<Json<ScrapeRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<ScrapeData>>, AppError> {
    let Json(mut req) = body.map_err(AppError::from)?;
    // `force_cloak` is `skip_deserializing`, so it is never read from the body.
    // The only source is this trusted, SaaS-set header (the engine is not
    // internet-exposed in the managed deployment), so a caller cannot force the
    // expensive cloak tier.
    req.force_cloak = Some(
        headers
            .get("x-crw-force-cloak")
            .and_then(|v| v.to_str().ok())
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false),
    );
    let parsed_url = url::Url::parse(&req.url)
        .map_err(|e| CrwError::InvalidRequest(format!("Invalid URL: {e}")))?;
    crw_core::url_safety::validate_safe_url_resolved(&parsed_url)
        .await
        .map_err(CrwError::InvalidRequest)?;
    validate_renderer_pin(req.renderer, req.render_js, &state)?;

    let llm_config = state.config.extraction.llm.as_ref();

    // Validate LLM-touching formats up front so we fail fast with a clear
    // message rather than running the full scrape and then erroring.
    if req.formats.contains(&OutputFormat::Summary)
        && llm_config.is_none()
        && req.llm_api_key.is_none()
    {
        return Err(AppError::from(CrwError::InvalidRequest(
            "summary format requires LLM config: set CRW_EXTRACTION__LLM__API_KEY \
             in server config or pass llm_api_key in the request body"
                .into(),
        )));
    }
    if let Some(cfg) = llm_config
        && let Some(header_name) = cfg.require_byok_header.as_deref()
        && (req.formats.contains(&OutputFormat::Summary)
            || req.formats.contains(&OutputFormat::Json))
        && req.llm_api_key.is_none()
    {
        // SaaS-fronted deploy: the request transformer must have set the
        // tenant header. Reject direct public callers.
        // NOTE: actually checking the header is done at the middleware
        // layer (see Phase 7a); this catches the missing-header case at
        // the request level when no BYOK key is supplied.
        let _ = header_name;
        return Err(AppError::from(CrwError::InvalidRequest(
            "LLM features require a per-request llm_api_key (BYOK header guard active)".into(),
        )));
    }

    let user_agent = &state.config.crawler.user_agent;
    let default_stealth =
        state.config.crawler.stealth.enabled && state.config.crawler.stealth.inject_headers;
    let deadline = Deadline::from_request_ms(
        state
            .config
            .effective_deadline_ms(req.deadline_ms, req.wait_for),
    );
    let data = scrape_url(
        &req,
        &state.renderer,
        llm_config,
        &state.config.extraction,
        user_agent,
        default_stealth,
        state.config.renderer.render_js_default,
        deadline,
    )
    .await?;

    // Return success:false when target HTTP status >= 400 and body is minimal.
    // Check all text-producing formats so rawHtml-only requests aren't falsely flagged.
    let status_code = data.metadata.status_code;
    if status_code >= 400 {
        let body_len = [
            data.markdown.as_deref(),
            data.plain_text.as_deref(),
            data.html.as_deref(),
            data.raw_html.as_deref(),
        ]
        .iter()
        .filter_map(|opt| opt.map(|t| t.len()))
        .max()
        .unwrap_or(0);

        if body_len < 200 {
            let error_msg = data
                .warning
                .clone()
                .unwrap_or_else(|| format!("Target returned HTTP {status_code}"));
            return Ok(Json(ApiResponse {
                success: false,
                data: Some(data),
                error: Some(error_msg),
                error_code: Some("http_error".into()),
                warning: None,
            }));
        }
    }

    // Anti-bot block: the verdict is now classified ONCE at the scrape choke
    // (`single::classify_block`) and stamped onto `data.block`, so v1/v2/crawl
    // share one decision. `error_code` stays "anti_bot" (identical to the prior
    // md_empty path) so credit-refund logic is unchanged.
    if let Some(b) = data.block.clone() {
        let error_msg = b.message();
        // Drop the interstitial/challenge shell so the caller gets a clean block
        // instead of the "Just a moment / Humans only" wall as markdown. Runs
        // after the http_error gate above, which reads the content lengths.
        let mut data = data;
        data.clear_body();
        return Ok(Json(ApiResponse {
            success: false,
            data: Some(data),
            error: Some(error_msg),
            error_code: Some("anti_bot".into()),
            warning: None,
        }));
    }

    // Promote data.warning to ApiResponse.warning so it's visible at top level
    let warning = data.warning.clone();
    let mut resp = ApiResponse::ok(data);
    resp.warning = warning;
    Ok(Json(resp))
}
