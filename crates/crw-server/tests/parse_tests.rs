//! Integration tests for `POST /v2/parse` (document upload → markdown).

use axum::http::StatusCode;
use axum_test::TestServer;
use axum_test::multipart::{MultipartForm, Part};
use crw_core::config::AppConfig;
use crw_server::app::create_app;
use crw_server::state::AppState;
use serde_json::Value;

const SAMPLE_PDF: &[u8] = include_bytes!("fixtures/sample.pdf");

fn test_app() -> TestServer {
    let config: AppConfig = toml::from_str("").unwrap();
    let state = AppState::new(config).expect("AppState::new failed");
    TestServer::new(create_app(state))
}

#[tokio::test]
async fn parse_pdf_upload_returns_markdown() {
    let s = test_app();
    let form = MultipartForm::new()
        .add_part(
            "file",
            Part::bytes(SAMPLE_PDF.to_vec())
                .file_name("sample.pdf")
                .mime_type("application/pdf"),
        )
        .add_text("options", r#"{"formats":["markdown"]}"#);

    let r = s.post("/v2/parse").multipart(form).await;
    r.assert_status(StatusCode::OK);
    let body: Value = r.json();
    assert_eq!(body["success"], true);
    let md = body["data"]["markdown"].as_str().unwrap_or("");
    assert!(
        md.contains("Hello fastCRW PDF parsing"),
        "expected parsed markdown, got: {md}"
    );
    assert_eq!(body["data"]["metadata"]["numPages"], 2);
    assert_eq!(body["data"]["metadata"]["sourceFilename"], "sample.pdf");
}

#[tokio::test]
async fn parse_rejects_non_pdf() {
    let s = test_app();
    let form = MultipartForm::new().add_part(
        "file",
        Part::bytes(b"<html>not a pdf</html>".to_vec())
            .file_name("x.html")
            .mime_type("text/html"),
    );
    let r = s.post("/v2/parse").multipart(form).await;
    r.assert_status(StatusCode::BAD_REQUEST);
    let body: Value = r.json();
    assert_eq!(body["success"], false);
}

#[tokio::test]
async fn parse_missing_file_400() {
    let s = test_app();
    let form = MultipartForm::new().add_text("options", "{}");
    let r = s.post("/v2/parse").multipart(form).await;
    r.assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn parse_corrupt_pdf_422() {
    let s = test_app();
    // Valid %PDF- magic so it passes the sniff, but a broken body → 422.
    let form = MultipartForm::new().add_part(
        "file",
        Part::bytes(b"%PDF-1.4\ngarbage body not a real pdf".to_vec())
            .file_name("broken.pdf")
            .mime_type("application/pdf"),
    );
    let r = s.post("/v2/parse").multipart(form).await;
    r.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
}
