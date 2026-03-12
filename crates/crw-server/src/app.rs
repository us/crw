use axum::Router;
use axum::body::Body;
use axum::extract::DefaultBodyLimit;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tower_http::cors::CorsLayer;
use tower_http::set_header::SetResponseHeaderLayer;
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
    let rate_limit_rps = state.config.server.rate_limit_rps;

    let api_routes = Router::new()
        .route(
            "/v1/scrape",
            post(routes::scrape::scrape).fallback(method_not_allowed),
        )
        .route(
            "/v1/crawl",
            post(routes::crawl::start_crawl).fallback(method_not_allowed),
        )
        .route(
            "/v1/crawl/{id}",
            get(routes::crawl::get_crawl)
                .delete(routes::crawl::cancel_crawl)
                .fallback(method_not_allowed),
        )
        .route(
            "/v1/map",
            post(routes::map::map).fallback(method_not_allowed),
        )
        .route(
            "/mcp",
            post(routes::mcp::mcp_handler).fallback(method_not_allowed),
        );

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

    let rate_limiter = if rate_limit_rps > 0 {
        Some(Arc::new(RateLimiter::new(rate_limit_rps)))
    } else {
        None
    };

    Router::new()
        .route(
            "/health",
            get(routes::health::health).fallback(method_not_allowed),
        )
        .with_state(state)
        .merge(api_routes)
        .layer(axum::middleware::from_fn(move |req, next| {
            let limiter = rate_limiter.clone();
            rate_limit_middleware(limiter, req, next)
        }))
        .layer(DefaultBodyLimit::max(MAX_BODY_SIZE))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            timeout,
        ))
        .layer(SetResponseHeaderLayer::overriding(
            axum::http::header::X_CONTENT_TYPE_OPTIONS,
            axum::http::HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            axum::http::header::X_FRAME_OPTIONS,
            axum::http::HeaderValue::from_static("DENY"),
        ))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
}

async fn method_not_allowed() -> impl IntoResponse {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        axum::Json(crw_core::types::ApiResponse::<()>::err_with_code(
            "Method not allowed",
            "method_not_allowed",
        )),
    )
}

/// Simple token-bucket rate limiter using atomic counters.
/// Refills `rps` tokens every second.
struct RateLimiter {
    tokens: AtomicU64,
    max_tokens: u64,
    last_refill: std::sync::Mutex<std::time::Instant>,
}

impl RateLimiter {
    fn new(rps: u64) -> Self {
        Self {
            tokens: AtomicU64::new(rps),
            max_tokens: rps,
            last_refill: std::sync::Mutex::new(std::time::Instant::now()),
        }
    }

    fn try_acquire(&self) -> bool {
        // Refill tokens based on elapsed time.
        {
            let mut last = self.last_refill.lock().unwrap();
            let elapsed = last.elapsed();
            if elapsed >= Duration::from_secs(1) {
                let refill = (elapsed.as_secs_f64() * self.max_tokens as f64) as u64;
                let current = self.tokens.load(Ordering::Relaxed);
                let new_val = (current + refill).min(self.max_tokens);
                self.tokens.store(new_val, Ordering::Relaxed);
                *last = std::time::Instant::now();
            }
        }

        // Try to consume one token.
        loop {
            let current = self.tokens.load(Ordering::Relaxed);
            if current == 0 {
                return false;
            }
            if self
                .tokens
                .compare_exchange_weak(current, current - 1, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return true;
            }
        }
    }
}

async fn rate_limit_middleware(
    limiter: Option<Arc<RateLimiter>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if let Some(limiter) = limiter
        && req.uri().path() != "/health"
        && !limiter.try_acquire()
    {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            axum::Json(crw_core::types::ApiResponse::<()>::err_with_code(
                "Rate limited",
                "rate_limited",
            )),
        )
            .into_response();
    }
    next.run(req).await
}
