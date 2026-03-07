use axum::Json;
use axum::extract::State;
use crw_core::error::CrwError;
use crw_core::types::{MapRequest, MapResponse};
use crw_crawl::crawl::{DiscoverOptions, discover_urls};

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
    crw_core::url_safety::validate_safe_url(&parsed_url).map_err(CrwError::InvalidRequest)?;

    let max_depth = req
        .max_depth
        .unwrap_or(state.config.crawler.default_max_depth);

    let urls = discover_urls(DiscoverOptions {
        base_url: &req.url,
        max_depth,
        use_sitemap: req.use_sitemap,
        renderer: &state.renderer,
        max_concurrency: state.config.crawler.max_concurrency,
        requests_per_second: state.config.crawler.requests_per_second,
        user_agent: &state.config.crawler.user_agent,
        proxy: state.config.crawler.proxy.clone(),
    })
    .await?;

    Ok(Json(MapResponse {
        success: true,
        links: urls,
    }))
}
