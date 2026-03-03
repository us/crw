use axum::Json;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use crw_core::types::ApiResponse;
use std::sync::Arc;

/// Constant-time byte comparison to prevent timing attacks.
/// Does not leak the length of valid keys via timing side-channels:
/// always iterates over the longer of the two inputs.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let len = a.len().max(b.len());
    let mut result = (a.len() != b.len()) as u8;
    for i in 0..len {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        result |= x ^ y;
    }
    result == 0
}

/// Exposed for integration tests only.
#[cfg(feature = "test-utils")]
pub fn constant_time_eq_pub(a: &[u8], b: &[u8]) -> bool {
    constant_time_eq(a, b)
}

/// Bearer token authentication middleware.
pub async fn auth_middleware(
    axum::extract::State(api_keys): axum::extract::State<Arc<Vec<String>>>,
    req: Request,
    next: Next,
) -> Response {
    if api_keys.is_empty() {
        return next.run(req).await;
    }

    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(header) if header.starts_with("Bearer ") => {
            let token = &header[7..];
            // Compare against all keys without short-circuiting to avoid
            // leaking which key index matched via timing.
            if api_keys.iter().fold(false, |found, k| {
                constant_time_eq(k.as_bytes(), token.as_bytes()) || found
            }) {
                next.run(req).await
            } else {
                let body = ApiResponse::<()>::err("Invalid API key");
                (StatusCode::UNAUTHORIZED, Json(body)).into_response()
            }
        }
        _ => {
            let body = ApiResponse::<()>::err("Missing Authorization header");
            (StatusCode::UNAUTHORIZED, Json(body)).into_response()
        }
    }
}
