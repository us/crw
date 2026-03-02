use axum::extract::{Path, State};
use axum::Json;
use crw_core::error::CrwError;
use crw_core::types::{CrawlRequest, CrawlStartResponse, CrawlState, CrawlStatus};
use crw_crawl::crawl::run_crawl;
use std::time::Instant;
use uuid::Uuid;

use crate::error::AppError;
use crate::state::{AppState, CrawlJob};

/// POST /v1/crawl — start a crawl job.
/// Response format matches Firecrawl: { success: true, id: "..." }
pub async fn start_crawl(
    State(state): State<AppState>,
    Json(req): Json<CrawlRequest>,
) -> Result<Json<CrawlStartResponse>, AppError> {
    let parsed_url = url::Url::parse(&req.url)
        .map_err(|e| CrwError::InvalidRequest(format!("Invalid URL: {e}")))?;
    if !matches!(parsed_url.scheme(), "http" | "https") {
        return Err(CrwError::InvalidRequest("Only http/https URLs are allowed".into()).into());
    }

    let id = Uuid::new_v4();
    let initial = CrawlState {
        id,
        status: CrawlStatus::InProgress,
        total: 0,
        completed: 0,
        data: vec![],
        error: None,
    };

    let (tx, rx) = tokio::sync::watch::channel(initial);

    {
        let mut jobs = state.crawl_jobs.write().await;
        jobs.insert(id, CrawlJob {
            rx,
            created_at: Instant::now(),
        });
    }

    let renderer = state.renderer.clone();
    let max_concurrency = state.config.crawler.max_concurrency;
    let respect_robots = state.config.crawler.respect_robots_txt;
    let rps = state.config.crawler.requests_per_second;
    let user_agent = state.config.crawler.user_agent.clone();
    let crawl_semaphore = state.crawl_semaphore.clone();
    let llm_config = state.config.extraction.llm.clone();

    tokio::spawn(async move {
        // Limit concurrent crawl jobs to prevent resource exhaustion.
        let _permit = match crawl_semaphore.acquire().await {
            Ok(p) => p,
            Err(_) => {
                let _ = tx.send(CrawlState {
                    id,
                    status: CrawlStatus::Failed,
                    total: 0,
                    completed: 0,
                    data: vec![],
                    error: Some("Server is overloaded, try again later".into()),
                });
                return;
            }
        };
        run_crawl(
            id,
            req,
            renderer,
            max_concurrency,
            respect_robots,
            rps,
            &user_agent,
            tx,
            llm_config.as_ref(),
        )
        .await;
    });

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
