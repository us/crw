use axum::Json;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use crw_core::types::ApiResponse;
use std::sync::Arc;

/// Constant-time byte comparison to prevent timing attacks.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
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
            if api_keys
                .iter()
                .any(|k| constant_time_eq(k.as_bytes(), token.as_bytes()))
            {
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
