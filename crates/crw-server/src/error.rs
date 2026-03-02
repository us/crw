use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use crw_core::error::CrwError;
use crw_core::types::ApiResponse;

/// Wrapper to implement IntoResponse for CrwError in the server crate.
pub struct AppError(pub CrwError);

impl From<CrwError> for AppError {
    fn from(e: CrwError) -> Self {
        Self(e)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self.0 {
            CrwError::InvalidRequest(_) => (StatusCode::BAD_REQUEST, self.0.to_string()),
            CrwError::NotFound(_) => (StatusCode::NOT_FOUND, self.0.to_string()),
            CrwError::Timeout(_) => (StatusCode::GATEWAY_TIMEOUT, self.0.to_string()),
            CrwError::HttpError(_) => (StatusCode::BAD_GATEWAY, self.0.to_string()),
            CrwError::ExtractionError(_) => (StatusCode::UNPROCESSABLE_ENTITY, self.0.to_string()),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, self.0.to_string()),
        };

        let body = ApiResponse::<()>::err(message);
        (status, Json(body)).into_response()
    }
}
