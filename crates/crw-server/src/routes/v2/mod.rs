//! Firecrawl `/v2/*` API surface (issue #62).
//!
//! v2 reuses the version-agnostic engine (`crw_crawl::single::scrape_url`,
//! `AppState::start_crawl_job`, `search_inner`, `discover_urls`); this module is
//! purely the HTTP serialization shell that adapts the v2 wire shapes
//! (object-formats, object map-links, paginated crawl status) to/from the
//! internal types. v1 is left byte-identical.

pub mod adapters;
pub mod batch;
pub mod crawl;
pub mod extract;
pub mod formats;
pub mod map;
pub mod parse;
pub mod scrape;
pub mod search;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};

use crate::routes::method_not_allowed;
use crate::state::AppState;

/// All `/v2/*` routes. Merged into `app.rs`'s `api_routes` before the shared
/// auth + rate-limit layers, so v2 inherits them for free.
///
/// `max_upload_bytes` is the effective `[document].max_upload_bytes` and becomes
/// the per-route body limit on `/v2/parse`. It is passed in (rather than read
/// from `AppState`) because the body-limit layer is installed at router-build
/// time, before state is attached. `/v1/capabilities` advertises the same value,
/// so the advertised upload cap is the enforced one.
pub fn router(max_upload_bytes: usize) -> Router<AppState> {
    Router::new()
        .route(
            "/v2/scrape",
            post(scrape::scrape).fallback(method_not_allowed),
        )
        .route(
            "/v2/scrape/{job_id}",
            get(scrape::get_scrape_job).fallback(method_not_allowed),
        )
        .route(
            "/v2/crawl",
            post(crawl::start_crawl).fallback(method_not_allowed),
        )
        .route(
            "/v2/crawl/active",
            get(crawl::active).fallback(method_not_allowed),
        )
        .route(
            "/v2/crawl/{id}",
            get(crawl::get_crawl)
                .delete(crawl::cancel_crawl)
                .fallback(method_not_allowed),
        )
        .route(
            "/v2/crawl/{id}/errors",
            get(crawl::get_errors).fallback(method_not_allowed),
        )
        .route(
            // File-upload parsing. Per-route 50 MB body limit overrides the
            // global 1 MB cap (innermost DefaultBodyLimit wins) — applied only
            // here so JSON endpoints stay DoS-bounded.
            "/v2/parse",
            post(parse::parse)
                .layer(DefaultBodyLimit::max(max_upload_bytes))
                .fallback(method_not_allowed),
        )
        .route(
            // Same handler as /v1/capabilities; v2 alias for SDK symmetry.
            "/v2/capabilities",
            get(crate::routes::capabilities::capabilities).fallback(method_not_allowed),
        )
        .route("/v2/map", post(map::map).fallback(method_not_allowed))
        .route(
            "/v2/search",
            post(search::search).fallback(method_not_allowed),
        )
        .route(
            // Batch submits carry up to `max_batch_urls` URLs — needs more
            // than the global 1 MB JSON cap (innermost DefaultBodyLimit wins).
            "/v2/batch/scrape",
            post(batch::start_batch)
                .layer(DefaultBodyLimit::max(batch::MAX_BATCH_BODY_BYTES))
                .fallback(method_not_allowed),
        )
        .route(
            "/v2/batch/scrape/{id}",
            get(batch::get_batch)
                .delete(batch::cancel_batch)
                .fallback(method_not_allowed),
        )
        .route(
            "/v2/batch/scrape/{id}/errors",
            get(batch::get_errors).fallback(method_not_allowed),
        )
        .route(
            "/v2/extract",
            post(extract::start_extract).fallback(method_not_allowed),
        )
        .route(
            "/v2/extract/{id}",
            get(extract::get_extract).fallback(method_not_allowed),
        )
}
