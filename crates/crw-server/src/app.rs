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
use crate::routes::{self, method_not_allowed};
use crate::state::AppState;

pub fn create_app(state: AppState) -> Router {
    let api_keys = Arc::new(state.config.auth.api_keys.clone());
    // Tower outer timeout. `effective_request_timeout_secs()` widens the
    // operator baseline so the longest legitimate handler runtime (auto-extended
    // scrape, search enrichment fan-out, map's 300s ceiling) isn't cancelled
    // by the outer layer before the inner deadline fires. See issue #35.
    let timeout = Duration::from_secs(state.config.effective_request_timeout_secs());
    let rate_limit_rps = state.config.server.rate_limit_rps;

    // Native vs Firecrawl-compat surface split.
    //
    // `/v1` (and, transitionally, `/v2`) are fastCRW's own API — free to evolve;
    // staying Firecrawl-compatible is a bonus, not a contract.
    //
    // The SAME handlers are ALSO mounted under `/firecrawl/*`, which is the
    // canonical, frozen Firecrawl drop-in surface (`/firecrawl/v1/*`,
    // `/firecrawl/v2/*`). New callers who want guaranteed Firecrawl-shape should
    // target `/firecrawl/*`; `/v2/*` at the root is a deprecated alias kept for
    // backward-compat. Nesting reuses the v1/v2 routers verbatim, so both
    // surfaces share request parsing, error envelopes, and body limits.
    //
    // All merged before the shared auth + rate-limit layers, so every surface
    // inherits auth, rate-limiting, body-limit and the timeout layer identically.
    // `/mcp` is version-less.
    // The `/v2/parse` body limit is installed at router-build time, so the
    // effective upload cap is resolved here from config and passed in. Both
    // surfaces get the same cap, and `/v1/capabilities` advertises that value.
    let max_upload_bytes = routes::v2::parse::effective_max_upload_bytes(&state.config);
    let firecrawl_compat = Router::new().nest(
        "/firecrawl",
        routes::v1::router().merge(routes::v2::router(max_upload_bytes)),
    );
    let api_routes = routes::v1::router()
        .merge(routes::v2::router(max_upload_bytes))
        .merge(firecrawl_compat)
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
        .route(
            "/openapi.json",
            get(routes::openapi::serve_openapi_3_1).fallback(method_not_allowed),
        )
        .route(
            "/openapi-3.0.json",
            get(routes::openapi::serve_openapi_3_0).fallback(method_not_allowed),
        )
        .route(
            "/ready",
            get(routes::health::ready).fallback(method_not_allowed),
        )
        .route(
            "/metrics",
            get(routes::metrics::metrics).fallback(method_not_allowed),
        )
        .route(
            "/metrics/renderer-breakers",
            get(routes::breakers::renderer_breakers).fallback(method_not_allowed),
        )
        .route(
            "/admin/breakers/reset",
            post(routes::breakers::reset_breakers).fallback(method_not_allowed),
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
        && req.uri().path() != "/ready"
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
