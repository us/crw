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
    assert!(json.get("active_crawl_jobs").is_some());
}

#[tokio::test]
async fn ready_endpoint_returns_renderers() {
    let server = test_app();
    let resp = server.get("/ready").await;
    // Status may be 200 or 503 depending on whether renderers reachable in
    // test env; we only assert the body shape carries renderer state.
    let json: serde_json::Value = resp.json();
    assert!(json["renderers"].is_object());
    assert!(json["status"].is_string());
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
async fn scrape_with_renderer_unavailable_returns_400() {
    // test_app uses default config (mode=auto with no CDP endpoints) → empty
    // JS pool. Pinning a specific renderer should fail-fast with 400 before
    // any network activity.
    let server = test_app();
    let resp = server
        .post("/v1/scrape")
        .json(&json!({"url": "https://example.com", "renderer": "chrome"}))
        .await;
    resp.assert_status(axum::http::StatusCode::BAD_REQUEST);
    let json: serde_json::Value = resp.json();
    assert_eq!(json["success"], false);
    let error = json["error"].as_str().unwrap();
    assert!(
        error.contains("renderer 'chrome' not available"),
        "expected pinned-renderer error, got: {error}"
    );
}

#[tokio::test]
async fn crawl_with_renderer_unavailable_returns_400() {
    // Crawl validates the pinned renderer before accepting the job — the
    // user gets HTTP 400 immediately rather than a queued-then-failed job.
    let server = test_app();
    let resp = server
        .post("/v1/crawl")
        .json(&json!({"url": "https://example.com", "renderer": "chrome"}))
        .await;
    resp.assert_status(axum::http::StatusCode::BAD_REQUEST);
    let json: serde_json::Value = resp.json();
    assert_eq!(json["success"], false);
    let error = json["error"].as_str().unwrap();
    assert!(
        error.contains("renderer 'chrome' not available"),
        "expected pinned-renderer error, got: {error}"
    );
}

#[tokio::test]
async fn crawl_with_renderer_auto_accepted() {
    // renderer:"auto" is the explicit-equivalent of omitting the field; should
    // not trigger availability validation.
    let server = test_app();
    let resp = server
        .post("/v1/crawl")
        .json(&json!({"url": "https://example.com", "renderer": "auto"}))
        .await;
    resp.assert_status_ok();
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

// ---------------------------------------------------------------------------
// Native /v1/batch/scrape — offline guards + cancel-terminal flow.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn v1_batch_requires_urls_400() {
    let s = test_app();
    let r = s.post("/v1/batch/scrape").json(&json!({"urls": []})).await;
    r.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn v1_batch_missing_urls_400() {
    let s = test_app();
    let r = s
        .post("/v1/batch/scrape")
        .json(&json!({"formats": ["markdown"]}))
        .await;
    r.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn v1_batch_all_invalid_urls_400() {
    // Unparseable URLs are rejected before any DNS lookup, so this stays offline.
    let s = test_app();
    let r = s
        .post("/v1/batch/scrape")
        .json(&json!({"urls": ["not-a-url", "also bad"]}))
        .await;
    r.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn v1_batch_exceeds_cap_400() {
    let config: AppConfig = toml::from_str("[crawler]\nmax_batch_urls = 2\n").unwrap();
    let state = AppState::new(config).expect("AppState::new failed");
    let s = TestServer::new(create_app(state));
    let r = s
        .post("/v1/batch/scrape")
        .json(&json!({"urls": ["not-a-url", "not-a-url", "not-a-url"]}))
        .await;
    r.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn v1_batch_status_unknown_404() {
    let s = test_app();
    let r = s
        .get("/v1/batch/scrape/00000000-0000-0000-0000-000000000000")
        .await;
    r.assert_status(axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn v1_batch_cancel_unknown_404() {
    let s = test_app();
    let r = s
        .delete("/v1/batch/scrape/00000000-0000-0000-0000-000000000000")
        .await;
    r.assert_status(axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn v1_batch_wrong_method_405() {
    let s = test_app();
    let r = s.get("/v1/batch/scrape").await;
    r.assert_status(axum::http::StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn v1_batch_cancel_flips_status_to_cancelled() {
    // Start the job at the state layer with a TEST-NET-1 blackhole IP so it
    // deterministically stays in progress until we cancel it.
    let config: AppConfig = toml::from_str("").unwrap();
    let state = AppState::new(config).expect("AppState::new failed");
    let s = TestServer::new(create_app(state.clone()));

    let template = crw_core::types::ScrapeRequest::default();
    let id = state
        .start_batch_job(vec!["http://192.0.2.1/".into()], template, None)
        .await;

    let r = s.delete(&format!("/v1/batch/scrape/{id}")).await;
    r.assert_status_ok();
    let body: serde_json::Value = r.json();
    assert_eq!(body["success"], true);

    // Status poll reports the terminal state, not "scraping".
    let g = s.get(&format!("/v1/batch/scrape/{id}")).await;
    g.assert_status_ok();
    let st: serde_json::Value = g.json();
    assert_eq!(st["status"], "cancelled");

    // Re-cancelling a finished job is rejected.
    let again = s.delete(&format!("/v1/batch/scrape/{id}")).await;
    again.assert_status(axum::http::StatusCode::BAD_REQUEST);
}
