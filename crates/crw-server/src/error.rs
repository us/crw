use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use crw_core::error::CrwError;
use crw_core::types::ApiResponse;

/// Wrapper to implement IntoResponse for CrwError in the server crate.
pub struct AppError(pub CrwError);

impl From<CrwError> for AppError {
    fn from(e: CrwError) -> Self {
        Self(e)
    }
}

impl From<JsonRejection> for AppError {
    fn from(rejection: JsonRejection) -> Self {
        Self(CrwError::InvalidRequest(rejection.body_text()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    fn status_for(err: CrwError) -> StatusCode {
        let app_err = AppError(err);
        let response = app_err.into_response();
        response.status()
    }

    #[test]
    fn app_error_invalid_request_400() {
        assert_eq!(
            status_for(CrwError::InvalidRequest("bad".into())),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn app_error_not_found_404() {
        assert_eq!(
            status_for(CrwError::NotFound("missing".into())),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn app_error_timeout_504() {
        assert_eq!(
            status_for(CrwError::Timeout(5000)),
            StatusCode::GATEWAY_TIMEOUT
        );
    }

    #[test]
    fn app_error_http_error_502() {
        assert_eq!(
            status_for(CrwError::HttpError("fail".into())),
            StatusCode::BAD_GATEWAY
        );
    }

    #[test]
    fn app_error_extraction_422() {
        assert_eq!(
            status_for(CrwError::ExtractionError("parse fail".into())),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[test]
    fn app_error_internal_500() {
        assert_eq!(
            status_for(CrwError::Internal("oops".into())),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn app_error_renderer_500() {
        assert_eq!(
            status_for(CrwError::RendererError("cdp fail".into())),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn app_error_body_is_api_response() {
        let app_err = AppError(CrwError::InvalidRequest("test error".into()));
        let response = app_err.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], false);
        assert!(json["error"].as_str().unwrap().contains("test error"));
    }
}
