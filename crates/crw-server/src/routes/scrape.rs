use axum::Json;
use axum::extract::State;
use crw_core::error::CrwError;
use crw_core::types::{ApiResponse, ScrapeData, ScrapeRequest};
use crw_crawl::single::scrape_url;

use crate::error::AppError;
use crate::state::AppState;

pub async fn scrape(
    State(state): State<AppState>,
    Json(req): Json<ScrapeRequest>,
) -> Result<Json<ApiResponse<ScrapeData>>, AppError> {
    let parsed_url = url::Url::parse(&req.url)
        .map_err(|e| CrwError::InvalidRequest(format!("Invalid URL: {e}")))?;
    crw_core::url_safety::validate_safe_url(&parsed_url)
        .map_err(CrwError::InvalidRequest)?;

    let llm_config = state.config.extraction.llm.as_ref();
    let data = scrape_url(&req, &state.renderer, llm_config).await?;
    Ok(Json(ApiResponse::ok(data)))
}
