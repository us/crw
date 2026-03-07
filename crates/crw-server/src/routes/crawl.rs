use axum::Json;
use axum::extract::{Path, State};
use crw_core::error::CrwError;
use crw_core::types::{CrawlRequest, CrawlStartResponse, CrawlState};
use uuid::Uuid;

use crate::error::AppError;
use crate::state::AppState;

/// POST /v1/crawl — start a crawl job.
/// Response format matches Firecrawl: { success: true, id: "..." }
pub async fn start_crawl(
    State(state): State<AppState>,
    Json(req): Json<CrawlRequest>,
) -> Result<Json<CrawlStartResponse>, AppError> {
    let parsed_url = url::Url::parse(&req.url)
        .map_err(|e| CrwError::InvalidRequest(format!("Invalid URL: {e}")))?;
    crw_core::url_safety::validate_safe_url(&parsed_url).map_err(CrwError::InvalidRequest)?;

    let id = state.start_crawl_job(req).await;

    Ok(Json(CrawlStartResponse {
        success: true,
        id: id.to_string(),
    }))
}

/// GET /v1/crawl/:id — get crawl status.
/// Response format matches Firecrawl: { status, total, completed, data, ... }
pub async fn get_crawl(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<CrawlState>, AppError> {
    let jobs = state.crawl_jobs.read().await;
    let job = jobs
        .get(&id)
        .ok_or_else(|| CrwError::NotFound(format!("Crawl job {id} not found")))?;

    let current = job.rx.borrow().clone();
    Ok(Json(current))
}
