use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use axum::Router;
use std::sync::Arc;
use std::time::Duration;
use tower_http::cors::CorsLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

/// Maximum request body size (1 MB) to prevent memory exhaustion from large payloads.
const MAX_BODY_SIZE: usize = 1024 * 1024;

use crate::middleware::auth_middleware;
use crate::routes;
use crate::state::AppState;

pub fn create_app(state: AppState) -> Router {
    let api_keys = Arc::new(state.config.auth.api_keys.clone());
    let timeout = Duration::from_secs(state.config.server.request_timeout_secs);

    let api_routes = Router::new()
        .route("/v1/scrape", post(routes::scrape::scrape))
        .route("/v1/crawl", post(routes::crawl::start_crawl))
        .route("/v1/crawl/:id", get(routes::crawl::get_crawl))
        .route("/v1/map", post(routes::map::map));

    let api_routes = if api_keys.is_empty() {
        api_routes.with_state(state.clone())
    } else {
        api_routes
            .route_layer(axum::middleware::from_fn_with_state(
                api_keys,
                auth_middleware,
            ))
            .with_state(state.clone())
    };

    Router::new()
        .route("/health", get(routes::health::health))
        .route("/mcp", post(routes::mcp::mcp_handler))
        .with_state(state)
        .merge(api_routes)
        .layer(DefaultBodyLimit::max(MAX_BODY_SIZE))
        .layer(TimeoutLayer::new(timeout))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
}
