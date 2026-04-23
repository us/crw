use axum_test::TestServer;
use crw_core::config::AppConfig;
use crw_server::app::create_app;
use crw_server::state::AppState;
use serde_json::json;

fn test_app() -> TestServer {
    let config: AppConfig = toml::from_str("").unwrap();
    let state = AppState::new(config).expect("AppState::new failed");
    let app = create_app(state);
    TestServer::new(app)
}

#[tokio::test]
async fn health_endpoint_returns_ok() {
    let server = test_app();
    let resp = server.get("/health").await;
    resp.assert_status_ok();
    let json: serde_json::Value = resp.json();
    assert_eq!(json["status"], "ok");
    assert!(json["version"].is_string());
    assert!(json["renderers"].is_object());
    assert!(json.get("active_crawl_jobs").is_some());
}

#[tokio::test]
async fn scrape_endpoint_invalid_url() {
    let server = test_app();
    let resp = server
        .post("/v1/scrape")
        .json(&json!({"url": "not-a-valid-url"}))
        .await;
    resp.assert_status(axum::http::StatusCode::BAD_REQUEST);
    let json: serde_json::Value = resp.json();
    assert_eq!(json["success"], false);
}

#[tokio::test]
async fn scrape_endpoint_ftp_url_rejected() {
    let server = test_app();
    let resp = server
        .post("/v1/scrape")
        .json(&json!({"url": "ftp://example.com/file"}))
        .await;
    resp.assert_status(axum::http::StatusCode::BAD_REQUEST);
    let json: serde_json::Value = resp.json();
    assert_eq!(json["success"], false);
    let error = json["error"].as_str().unwrap();
    assert!(
        error.contains("http") || error.contains("https"),
        "Error should mention allowed schemes. Got: {error}"
    );
}

#[tokio::test]
async fn crawl_start_returns_job_id() {
    let server = test_app();
    let resp = server
        .post("/v1/crawl")
        .json(&json!({"url": "https://example.com"}))
        .await;
    resp.assert_status_ok();
    let json: serde_json::Value = resp.json();
    assert_eq!(json["success"], true);
    let id = json["id"].as_str().unwrap();
    // Should be a valid UUID
    assert!(
        uuid::Uuid::parse_str(id).is_ok(),
        "ID should be valid UUID: {id}"
    );
}

#[tokio::test]
async fn crawl_status_not_found() {
    let server = test_app();
    let random_uuid = uuid::Uuid::new_v4();
    let resp = server.get(&format!("/v1/crawl/{random_uuid}")).await;
    resp.assert_status(axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn crawl_start_invalid_url() {
    let server = test_app();
    let resp = server
        .post("/v1/crawl")
        .json(&json!({"url": "not-valid"}))
        .await;
    resp.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn map_endpoint_invalid_url() {
    let server = test_app();
    let resp = server
        .post("/v1/map")
        .json(&json!({"url": "ftp://bad.com"}))
        .await;
    resp.assert_status(axum::http::StatusCode::BAD_REQUEST);
}
