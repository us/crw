use axum::extract::State;
use axum::Json;
use crw_core::error::CrwError;
use crw_core::types::{MapRequest, MapResponse};
use crw_crawl::crawl::discover_urls;

use crate::error::AppError;
use crate::state::AppState;

/// POST /v1/map — discover URLs.
/// Response format matches Firecrawl: { success: true, links: [...] }
pub async fn map(
    State(state): State<AppState>,
    Json(req): Json<MapRequest>,
) -> Result<Json<MapResponse>, AppError> {
    let parsed_url = url::Url::parse(&req.url)
        .map_err(|e| CrwError::InvalidRequest(format!("Invalid URL: {e}")))?;
    if !matches!(parsed_url.scheme(), "http" | "https") {
        return Err(CrwError::InvalidRequest("Only http/https URLs are allowed".into()).into());
    }

    let max_depth = req
        .max_depth
        .unwrap_or(state.config.crawler.default_max_depth);

    let urls = discover_urls(
        &req.url,
        max_depth,
        req.use_sitemap,
        &state.renderer,
        state.config.crawler.max_concurrency,
        state.config.crawler.requests_per_second,
        &state.config.crawler.user_agent,
    )
    .await?;

    Ok(Json(MapResponse {
        success: true,
        links: urls,
    }))
}
