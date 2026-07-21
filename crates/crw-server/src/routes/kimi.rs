//! `/kimi/*` compatibility surface for Kimi Code's built-in `moonshot_search`
//! and `moonshot_fetch` tools. Two thin translators that reshape the native
//! `/v1` search/scrape into Kimi's snake_case wire format:
//!
//! - `POST /kimi/search` `{text_query}` → `{search_results:[{title,url,snippet,date,site_name}]}`
//! - `POST /kimi/fetch`  `{url}` → raw `text/markdown` body; a failure surfaces
//!   as a non-200 (fetch inherits scrape's `http_error`/anti-bot heuristic, not a
//!   bare HTTP-status check, so a large soft-error page can still return 200).
//!
//! No new business logic: search reuses `search::search_inner`, fetch reuses
//! the `scrape::scrape` handler verbatim (SSRF `validate_safe_url_resolved` +
//! anti-bot envelope). The snake_case wire is a deliberate foreign shape,
//! isolated in the `Kimi*` structs so the camelCase native types stay untouched.

use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crw_core::error::CrwError;
use crw_core::types::{ScrapeRequest, SearchData, SearchRequest, SearchResult};

use crate::error::AppError;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/kimi/search",
            post(kimi_search).fallback(crate::routes::method_not_allowed),
        )
        .route(
            "/kimi/fetch",
            post(kimi_fetch).fallback(crate::routes::method_not_allowed),
        )
}

/// Kimi search request. `text_query` required; `limit` honored (kimi-cli sends
/// it). Unknown fields (`enable_page_crawling`, `timeout_seconds`) are ignored
/// by serde's default permissive parsing.
#[derive(Deserialize)]
struct KimiSearchBody {
    text_query: String,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Serialize)]
struct KimiSearchResult {
    title: String,
    url: String,
    snippet: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    date: Option<String>,
    site_name: String,
}

#[derive(Serialize)]
struct KimiSearchResponse {
    search_results: Vec<KimiSearchResult>,
}

#[derive(Deserialize)]
struct KimiFetchBody {
    url: String,
}

/// Host of `url` with a leading `www.` stripped; empty string if unparseable.
fn site_name(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
        .map(|h| h.strip_prefix("www.").map(str::to_string).unwrap_or(h))
        .unwrap_or_default()
}

async fn kimi_search(
    State(state): State<AppState>,
    body: Result<Json<KimiSearchBody>, JsonRejection>,
) -> Result<Json<KimiSearchResponse>, AppError> {
    let Json(body) = body.map_err(AppError::from)?;
    let limit = body.limit.unwrap_or(10);
    let req: SearchRequest =
        serde_json::from_value(json!({ "query": body.text_query, "limit": limit }))
            .map_err(|e| CrwError::InvalidRequest(format!("Invalid search request: {e}")))?;

    let resp = crate::routes::search::search_inner(&state, req).await?;

    // Kimi search is flat; grouped never occurs (we never set `sources`) but
    // flatten defensively to the `web` bucket.
    let rows: Vec<SearchResult> = match resp.data.map(|d| d.results) {
        Some(SearchData::Flat(v)) => v,
        Some(SearchData::Grouped(g)) => g.web.unwrap_or_default(),
        None => Vec::new(),
    };

    let search_results = rows
        .into_iter()
        .map(|r| {
            let site = site_name(&r.url);
            let snippet = if r.snippet.is_empty() {
                r.description
            } else {
                r.snippet
            };
            KimiSearchResult {
                title: r.title,
                url: r.url,
                snippet,
                date: r.published_date,
                site_name: site,
            }
        })
        .collect();

    Ok(Json(KimiSearchResponse { search_results }))
}

async fn kimi_fetch(
    State(state): State<AppState>,
    body: Result<Json<KimiFetchBody>, JsonRejection>,
) -> Response {
    let req = match body {
        Ok(Json(b)) => b,
        Err(e) => return AppError::from(e).into_response(),
    };

    let scrape_req: ScrapeRequest =
        match serde_json::from_value(json!({ "url": req.url, "formats": ["markdown"] })) {
            Ok(r) => r,
            Err(e) => {
                return AppError::from(CrwError::InvalidRequest(format!(
                    "Invalid fetch request: {e}"
                )))
                .into_response();
            }
        };

    // Reuse the scrape handler verbatim: SSRF validation + anti-bot envelope.
    // Empty headers -> no `x-crw-force-cloak` -> force_cloak stays off for this
    // internal reuse (the hint is only ever set by the SaaS front-end).
    match crate::routes::scrape::scrape(
        State(state),
        axum::http::HeaderMap::new(),
        Ok(Json(scrape_req)),
    )
    .await
    {
        Err(e) => e.into_response(),
        Ok(Json(resp)) => match resp.data.and_then(|d| d.markdown) {
            Some(md) if resp.success && !md.is_empty() => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
                md,
            )
                .into_response(),
            // success:false (block/http_error) or empty/missing markdown → error.
            _ => {
                let err = resp.error.unwrap_or_else(|| "fetch failed".to_string());
                (StatusCode::BAD_GATEWAY, err).into_response()
            }
        },
    }
}
