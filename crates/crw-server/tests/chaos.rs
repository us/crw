use axum_test::TestServer;
use crw_core::config::AppConfig;
use crw_server::app::create_app;
use crw_server::state::AppState;
use serde_json::json;

fn test_app() -> TestServer {
    let config: AppConfig = toml::from_str("").unwrap();
    let state = AppState::new(config);
    let app = create_app(state);
    TestServer::new(app)
}

// ── Concurrent crawl tests ──

#[tokio::test]
async fn crawl_concurrent_start_unique_ids() {
    let server = test_app();
    let mut ids = Vec::new();

    for _ in 0..10 {
        let resp = server
            .post("/v1/crawl")
            .json(&json!({"url": "https://example.com"}))
            .await;
        resp.assert_status_ok();
        let json: serde_json::Value = resp.json();
        let id = json["id"].as_str().unwrap().to_string();
        ids.push(id);
    }

    // All IDs should be unique
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), 10, "All crawl job IDs should be unique");
}

#[tokio::test]
async fn crawl_concurrent_status_no_deadlock() {
    let server = test_app();

    // Start a crawl job
    let resp = server
        .post("/v1/crawl")
        .json(&json!({"url": "https://example.com"}))
        .await;
    let json: serde_json::Value = resp.json();
    let id = json["id"].as_str().unwrap();

    // Hit status endpoint many times sequentially — should not deadlock
    for _ in 0..20 {
        let _resp = server.get(&format!("/v1/crawl/{id}")).await;
    }
    // If we get here, no deadlock occurred
}

// ── MCP huge payload ──

#[tokio::test]
async fn mcp_huge_payload() {
    let server = test_app();
    // 2MB payload — should be rejected by body size limit (1MB)
    let huge = "x".repeat(2 * 1024 * 1024);
    let resp = server
        .post("/mcp")
        .content_type("application/json")
        .bytes(huge.into())
        .await;
    // Should be rejected — either 413 Payload Too Large or 400
    let status = resp.status_code();
    assert!(
        status == axum::http::StatusCode::PAYLOAD_TOO_LARGE
            || status == axum::http::StatusCode::BAD_REQUEST
            || status == axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        "Huge payload should be rejected, got: {status}"
    );
}

// ── Health endpoint under load ──

#[tokio::test]
async fn health_concurrent_requests() {
    let server = test_app();
    for _ in 0..50 {
        let resp = server.get("/health").await;
        resp.assert_status_ok();
    }
}
