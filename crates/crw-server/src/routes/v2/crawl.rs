//! `POST /v2/crawl`, `GET/DELETE /v2/crawl/{id}`, `GET /v2/crawl/active`,
//! `GET /v2/crawl/{id}/errors`. Reuses the existing in-memory crawl-job engine
//! (`AppState::start_crawl_job` + `crawl_jobs`); only the wire shapes differ.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crw_core::error::CrwError;
use crw_core::types::{CrawlRequest, CrawlStatus, OutputFormat, RequestedRenderer};

use super::adapters::{DEFAULT_PAGE_LIMIT, V2CrawlStatus, build_crawl_status};
use super::formats::{FormatSpec, decompose};
use crate::error::AppError;
use crate::state::{AppState, validate_crawl_renderer};

/// Derive the public scheme+host for `next`/`url` from the inbound request.
/// Matches Firecrawl (uses the request Host). Behind the SaaS proxy the path is
/// rewritten there; the SDK overrides the host anyway, keeping only path+query.
pub(crate) fn base_url(headers: &HeaderMap) -> String {
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("http");
    format!("{scheme}://{host}")
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct V2CrawlRequest {
    pub url: String,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub max_discovery_depth: Option<u32>,
    /// Nested per-page scrape options. We thread `formats`/`onlyMainContent`/
    /// `waitFor` through to the engine (not just tolerate them).
    #[serde(default)]
    pub scrape_options: Option<Value>,
    #[serde(default)]
    pub renderer: Option<RequestedRenderer>,
    #[serde(default)]
    pub country: Option<String>,
    /// BYOP proxy pool (crw extension), rotated per `proxy_rotation`. Accepts the
    /// snake_case `proxy_list` alias (what the managed layer injects).
    #[serde(default, alias = "proxy_list")]
    pub proxy_list: Vec<String>,
    #[serde(default, alias = "proxy_rotation")]
    pub proxy_rotation: Option<crw_core::proxy::ProxyRotation>,
}

#[derive(Debug, Serialize)]
pub struct V2CrawlStartResponse {
    pub success: bool,
    pub id: String,
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub skip: Option<usize>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Internal projection of a v2 `scrapeOptions` object.
pub(crate) struct ScrapeOpts {
    pub formats: Vec<OutputFormat>,
    pub json_schema: Option<Value>,
    pub only_main_content: bool,
    pub wait_for: Option<u64>,
}

/// Pull the internal scrape projection out of a v2 `scrapeOptions` object.
pub(crate) fn scrape_opts_to_internal(opts: &Option<Value>) -> Result<ScrapeOpts, CrwError> {
    let mut out = ScrapeOpts {
        formats: vec![OutputFormat::Markdown],
        json_schema: None,
        only_main_content: true,
        wait_for: None,
    };
    if let Some(Value::Object(m)) = opts {
        if let Some(f) = m.get("formats") {
            let specs: Vec<FormatSpec> = serde_json::from_value(f.clone()).map_err(|e| {
                CrwError::InvalidRequest(format!("invalid scrapeOptions.formats: {e}"))
            })?;
            let d = decompose(&specs).map_err(CrwError::InvalidRequest)?;
            out.formats = d.formats;
            out.json_schema = d.json_schema;
        }
        if let Some(b) = m.get("onlyMainContent").and_then(Value::as_bool) {
            out.only_main_content = b;
        }
        if let Some(w) = m.get("waitFor").and_then(Value::as_u64) {
            out.wait_for = Some(w);
        }
    }
    Ok(out)
}

pub async fn start_crawl(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<V2CrawlRequest>, JsonRejection>,
) -> Result<Json<V2CrawlStartResponse>, AppError> {
    let Json(v2) = body.map_err(AppError::from)?;
    let parsed_url = url::Url::parse(&v2.url)
        .map_err(|e| CrwError::InvalidRequest(format!("Invalid URL: {e}")))?;
    crw_core::url_safety::validate_safe_url_resolved(&parsed_url)
        .await
        .map_err(CrwError::InvalidRequest)?;

    let opts = scrape_opts_to_internal(&v2.scrape_options)?;
    let req = CrawlRequest {
        url: v2.url.clone(),
        max_depth: v2.max_discovery_depth,
        max_pages: v2.limit,
        formats: opts.formats,
        only_main_content: opts.only_main_content,
        json_schema: opts.json_schema,
        render_js: None,
        wait_for: opts.wait_for,
        renderer: v2.renderer,
        country: v2.country,
        proxy_list: v2.proxy_list,
        proxy_rotation: v2.proxy_rotation,
    };
    validate_crawl_renderer(&req, &state)?;

    let id = state.start_crawl_job(req).await;
    let base = base_url(&headers);
    Ok(Json(V2CrawlStartResponse {
        success: true,
        id: id.to_string(),
        url: format!("{base}/v2/crawl/{id}"),
    }))
}

pub async fn get_crawl(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(page): Query<PageQuery>,
) -> Result<Json<V2CrawlStatus>, AppError> {
    let (snapshot, created_at) = {
        let jobs = state.crawl_jobs.read().await;
        let job = jobs
            .get(&id)
            .ok_or_else(|| CrwError::NotFound(format!("Crawl job {id} not found")))?;
        (job.rx.borrow().clone(), job.created_at)
    };
    let skip = page.skip.unwrap_or(0);
    let limit = page.limit.unwrap_or(DEFAULT_PAGE_LIMIT);
    let base = base_url(&headers);
    let status = build_crawl_status(
        &snapshot,
        created_at,
        state.config.crawler.job_ttl_secs,
        skip,
        limit,
        &base,
        "/v2/crawl",
        id,
        "basic",
    );
    Ok(Json(status))
}

pub async fn cancel_crawl(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let mut jobs = state.crawl_jobs.write().await;
    let job = jobs
        .get_mut(&id)
        .ok_or_else(|| CrwError::NotFound(format!("Crawl job {id} not found")))?;
    let status = job.rx.borrow().status;
    if matches!(
        status,
        CrawlStatus::Completed | CrawlStatus::Failed | CrawlStatus::Cancelled
    ) {
        return Err(AppError(CrwError::InvalidRequest(
            "Crawl job already finished".into(),
        )));
    }
    // Abort, then mark terminal — otherwise polls return "scraping" until
    // TTL eviction and SDK waiters hang.
    if let Some(handle) = job.abort_handle.take() {
        handle.abort();
    }
    job.tx.send_modify(|st| st.status = CrawlStatus::Cancelled);
    Ok(Json(serde_json::json!({
        "success": true,
        "status": "cancelled",
        "message": format!("Crawl job {id} cancelled"),
    })))
}

/// `GET /v2/crawl/active` (Tier-3) — list still-running job ids.
pub async fn active(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let jobs = state.crawl_jobs.read().await;
    let ids: Vec<String> = jobs
        .iter()
        .filter(|(_, j)| matches!(j.rx.borrow().status, CrawlStatus::InProgress))
        .map(|(id, _)| id.to_string())
        .collect();
    Ok(Json(serde_json::json!({ "success": true, "crawls": ids })))
}

/// `GET /v2/crawl/{id}/errors` (Tier-3).
pub async fn get_errors(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let jobs = state.crawl_jobs.read().await;
    let job = jobs
        .get(&id)
        .ok_or_else(|| CrwError::NotFound(format!("Crawl job {id} not found")))?;
    let err = job.rx.borrow().error.clone();
    let errors: Vec<Value> = err
        .into_iter()
        .map(|e| serde_json::json!({ "id": id.to_string(), "error": e }))
        .collect();
    Ok(Json(
        serde_json::json!({ "success": true, "errors": errors, "robotsBlocked": [] }),
    ))
}
