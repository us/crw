//! `POST /v2/batch/scrape` (+ status/cancel/errors). Reuses the crawl-job
//! machinery over an explicit URL list (see `AppState::start_batch_job`). The
//! status envelope is identical to `/v2/crawl/{id}` (the live API matches).

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use futures::StreamExt;
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crw_core::error::CrwError;

use super::adapters::{DEFAULT_PAGE_LIMIT, V2CrawlStatus, build_crawl_status};
use super::crawl::{PageQuery, base_url};
use super::scrape::{V2ScrapeRequest, to_internal};
use crate::error::AppError;
use crate::state::AppState;

/// Per-route body cap for batch submits: a 10k-URL list at a few hundred
/// bytes per URL overflows the global 1 MB JSON cap; 8 MB keeps 10k long
/// URLs submittable while staying DoS-bounded.
pub const MAX_BATCH_BODY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct V2BatchStartResponse {
    pub success: bool,
    pub id: String,
    pub url: String,
    // Firecrawl spells this `invalidURLs` (capital URL); camelCase would give
    // `invalidUrls`, which the SDK's key-based normalization wouldn't match.
    #[serde(rename = "invalidURLs")]
    pub invalid_urls: Vec<String>,
}

pub async fn start_batch(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Result<Json<V2BatchStartResponse>, AppError> {
    let Json(mut raw) = body.map_err(AppError::from)?;
    let obj = raw
        .as_object_mut()
        .ok_or_else(|| CrwError::InvalidRequest("request body must be a JSON object".into()))?;

    let urls_val = obj
        .remove("urls")
        .ok_or_else(|| CrwError::InvalidRequest("`urls` is required".into()))?;
    let urls: Vec<String> = serde_json::from_value(urls_val)
        .map_err(|e| CrwError::InvalidRequest(format!("invalid `urls`: {e}")))?;
    if urls.is_empty() {
        return Err(AppError::from(CrwError::InvalidRequest(
            "`urls` must contain at least one URL".into(),
        )));
    }
    let max_urls = state.config.crawler.max_batch_urls;
    if urls.len() > max_urls {
        return Err(AppError::from(CrwError::InvalidRequest(format!(
            "`urls` exceeds the maximum of {max_urls} URLs per batch (got {})",
            urls.len()
        ))));
    }
    let ignore_invalid = obj
        .get("ignoreInvalidURLs")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    // Build the per-page scrape template from the remaining (base scrape)
    // options by reusing the v2 scrape request → internal conversion. A
    // placeholder URL satisfies the required field; it's cleared afterward.
    obj.insert(
        "url".to_string(),
        Value::String("https://placeholder.invalid/".into()),
    );
    let template_v2: V2ScrapeRequest = serde_json::from_value(Value::Object(obj.clone()))
        .map_err(|e| CrwError::InvalidRequest(format!("invalid batch scrape options: {e}")))?;
    let (mut template, _decomposed, _tier) = to_internal(template_v2)?;
    template.url = String::new();

    // Partition URLs into valid / invalid (SSRF-checked, same as v1 scrape).
    // Validation resolves DNS per URL — run it with bounded concurrency so a
    // 10k-URL batch submits in seconds instead of minutes, preserving input
    // order via `buffered`.
    const VALIDATE_CONCURRENCY: usize = 64;
    let checks = futures::stream::iter(urls.into_iter().map(|u| async move {
        let ok = match url::Url::parse(&u) {
            Ok(parsed) => crw_core::url_safety::validate_safe_url_resolved(&parsed)
                .await
                .is_ok(),
            Err(_) => false,
        };
        (u, ok)
    }))
    .buffered(VALIDATE_CONCURRENCY)
    .collect::<Vec<_>>()
    .await;
    let mut valid = Vec::new();
    let mut invalid = Vec::new();
    for (u, ok) in checks {
        if ok { valid.push(u) } else { invalid.push(u) }
    }
    if valid.is_empty() {
        return Err(AppError::from(CrwError::InvalidRequest(
            "no valid URLs to scrape".into(),
        )));
    }
    if !invalid.is_empty() && !ignore_invalid {
        return Err(AppError::from(CrwError::InvalidRequest(format!(
            "invalid URLs: {}",
            invalid.join(", ")
        ))));
    }

    let id = state.start_batch_job(valid, template).await;
    let base = base_url(&headers);
    Ok(Json(V2BatchStartResponse {
        success: true,
        id: id.to_string(),
        url: format!("{base}/v2/batch/scrape/{id}"),
        invalid_urls: invalid,
    }))
}

pub async fn get_batch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(page): Query<PageQuery>,
) -> Result<Json<V2CrawlStatus>, AppError> {
    let (snapshot, created_at) = {
        let jobs = state.crawl_jobs.read().await;
        let job = jobs
            .get(&id)
            .ok_or_else(|| CrwError::NotFound(format!("Batch job {id} not found")))?;
        (job.rx.borrow().clone(), job.created_at)
    };
    let skip = page.skip.unwrap_or(0);
    let limit = page.limit.unwrap_or(DEFAULT_PAGE_LIMIT);
    let base = base_url(&headers);
    Ok(Json(build_crawl_status(
        &snapshot,
        created_at,
        state.config.crawler.job_ttl_secs,
        skip,
        limit,
        &base,
        "/v2/batch/scrape",
        id,
        "basic",
    )))
}

pub async fn cancel_batch(state: State<AppState>, id: Path<Uuid>) -> Result<Json<Value>, AppError> {
    super::crawl::cancel_crawl(state, id).await
}

pub async fn get_errors(state: State<AppState>, id: Path<Uuid>) -> Result<Json<Value>, AppError> {
    super::crawl::get_errors(state, id).await
}
