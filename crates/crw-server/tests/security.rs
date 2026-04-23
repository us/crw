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

fn test_app_with_auth() -> TestServer {
    let config: AppConfig = toml::from_str(
        r#"
[auth]
api_keys = ["secret-key"]
"#,
    )
    .unwrap();
    let state = AppState::new(config).expect("AppState::new failed");
    let app = create_app(state);
    TestServer::new(app)
}

// ── SSRF Prevention Tests ──

#[tokio::test]
async fn ssrf_file_protocol_blocked() {
    let server = test_app();
    let resp = server
        .post("/v1/scrape")
        .json(&json!({"url": "file:///etc/passwd"}))
        .await;
    resp.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn ssrf_ftp_protocol_blocked() {
    let server = test_app();
    let resp = server
        .post("/v1/scrape")
        .json(&json!({"url": "ftp://evil.com/data"}))
        .await;
    resp.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn ssrf_gopher_protocol_blocked() {
    let server = test_app();
    let resp = server
        .post("/v1/scrape")
        .json(&json!({"url": "gopher://evil.com/"}))
        .await;
    resp.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn ssrf_data_protocol_blocked() {
    let server = test_app();
    let resp = server
        .post("/v1/scrape")
        .json(&json!({"url": "data:text/html,<h1>XSS</h1>"}))
        .await;
    resp.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn ssrf_localhost_blocked() {
    let server = test_app();
    let resp = server
        .post("/v1/scrape")
        .json(&json!({"url": "http://localhost:9999/secret"}))
        .await;
    resp.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn ssrf_127_0_0_1_blocked() {
    let server = test_app();
    let resp = server
        .post("/v1/scrape")
        .json(&json!({"url": "http://127.0.0.1:8080/secret"}))
        .await;
    resp.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn ssrf_aws_metadata_169_254_blocked() {
    let server = test_app();
    let resp = server
        .post("/v1/scrape")
        .json(&json!({"url": "http://169.254.169.254/latest/meta-data/"}))
        .await;
    resp.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn ssrf_private_ip_10_blocked() {
    let server = test_app();
    let resp = server
        .post("/v1/scrape")
        .json(&json!({"url": "http://10.0.0.1/internal"}))
        .await;
    resp.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn ssrf_private_ip_192_168_blocked() {
    let server = test_app();
    let resp = server
        .post("/v1/scrape")
        .json(&json!({"url": "http://192.168.1.1/admin"}))
        .await;
    resp.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

// ── Auth Security ──

#[tokio::test]
async fn auth_mcp_endpoint_protected() {
    let server = test_app_with_auth();
    // Without auth token, MCP should be rejected
    let resp = server
        .post("/mcp")
        .content_type("application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "ping"
        }))
        .await;
    resp.assert_status(axum::http::StatusCode::UNAUTHORIZED);
}

// ── Information Disclosure Tests ──

#[tokio::test]
async fn error_response_no_stack_trace() {
    let server = test_app();
    let resp = server
        .post("/v1/scrape")
        .json(&json!({"url": "not-valid"}))
        .await;
    let body = resp.text();
    assert!(
        !body.contains("at "),
        "Error response should not contain stack traces"
    );
    assert!(
        !body.contains("thread '"),
        "Error response should not contain thread info"
    );
}

#[tokio::test]
async fn error_response_no_internal_paths() {
    let server = test_app();
    let resp = server
        .post("/v1/scrape")
        .json(&json!({"url": "not-valid"}))
        .await;
    let body = resp.text();
    assert!(
        !body.contains("/Users/"),
        "Error response should not contain filesystem paths"
    );
    assert!(
        !body.contains("crates/"),
        "Error response should not contain crate paths"
    );
}

// ── Crawl SSRF ──

#[tokio::test]
async fn crawl_ssrf_ftp_blocked() {
    let server = test_app();
    let resp = server
        .post("/v1/crawl")
        .json(&json!({"url": "ftp://evil.com/"}))
        .await;
    resp.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn map_ssrf_file_blocked() {
    let server = test_app();
    let resp = server
        .post("/v1/map")
        .json(&json!({"url": "file:///etc/passwd"}))
        .await;
    resp.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

// ── CSS Selector Injection ──

#[test]
fn css_selector_injection_wildcard() {
    // * selector should not crash
    let html = "<body><p>Content</p></body>";
    let result = crw_extract::clean::clean_html(html, false, &["*".into()], &[]);
    assert!(result.is_ok(), "Wildcard CSS selector should not crash");
}
