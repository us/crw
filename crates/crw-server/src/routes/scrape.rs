use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use crw_core::Deadline;
use crw_core::error::CrwError;
use crw_core::types::{ApiResponse, ScrapeData, ScrapeRequest};
use crw_crawl::single::scrape_url;

use crate::error::AppError;
use crate::state::{AppState, validate_renderer_pin};

pub async fn scrape(
    State(state): State<AppState>,
    body: Result<Json<ScrapeRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<ScrapeData>>, AppError> {
    let Json(req) = body.map_err(AppError::from)?;
    let parsed_url = url::Url::parse(&req.url)
        .map_err(|e| CrwError::InvalidRequest(format!("Invalid URL: {e}")))?;
    crw_core::url_safety::validate_safe_url(&parsed_url).map_err(CrwError::InvalidRequest)?;
    validate_renderer_pin(req.renderer, req.render_js, &state)?;

    let llm_config = state.config.extraction.llm.as_ref();
    let user_agent = &state.config.crawler.user_agent;
    let default_stealth =
        state.config.crawler.stealth.enabled && state.config.crawler.stealth.inject_headers;
    let deadline = Deadline::from_request_ms(
        req.deadline_ms
            .unwrap_or(state.config.request.deadline_ms_default),
    );
    let data = scrape_url(
        &req,
        &state.renderer,
        llm_config,
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

    // Anti-bot interstitial detection: if extraction produced no real content
    // AND a block warning was attached, surface as a failure with the proper
    // error code instead of returning empty markdown with success=true. Bench
    // analysis: ~19/1000 URLs (Reuters, Forbes, Inc, Zoro, WSJ, …) hit this
    // path through DataDome/PerimeterX/Akamai challenges.
    let md_empty = data
        .markdown
        .as_deref()
        .map(|s| s.trim().len() < 100)
        .unwrap_or(true);
    let blocked = data
        .warning
        .as_deref()
        .map(|w| w.starts_with("Blocked by anti-bot"))
        .unwrap_or(false);
    if md_empty && blocked {
        let error_msg = data
            .warning
            .clone()
            .unwrap_or_else(|| "Blocked by anti-bot protection".into());
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
