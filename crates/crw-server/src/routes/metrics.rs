use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

pub async fn metrics() -> Response {
    let body = crw_core::metrics::gather_text();
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; version=0.0.4"),
        )],
        body,
    )
        .into_response()
}
