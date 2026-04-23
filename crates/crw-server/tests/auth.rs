use axum::http::HeaderValue;
use axum_test::TestServer;
use crw_core::config::AppConfig;
use crw_server::app::create_app;
use crw_server::state::AppState;
use serde_json::json;

fn test_app_with_auth(api_keys: Vec<String>) -> TestServer {
    let toml_str = format!(
        r#"
[auth]
api_keys = {:?}
"#,
        api_keys
    );
    let config: AppConfig = toml::from_str(&toml_str).unwrap();
    let state = AppState::new(config).expect("AppState::new failed");
    let app = create_app(state);
    TestServer::new(app)
}

fn test_app_no_auth() -> TestServer {
    let config: AppConfig = toml::from_str("").unwrap();
    let state = AppState::new(config).expect("AppState::new failed");
    let app = create_app(state);
    TestServer::new(app)
}

#[tokio::test]
async fn auth_valid_bearer_token() {
    let server = test_app_with_auth(vec!["test-key-123".into()]);
    let resp = server
        .post("/v1/scrape")
        .add_header(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer test-key-123"),
        )
        .json(&json!({"url": "https://example.com"}))
        .await;
    // Should not be 401 — might be 502 (can't reach example.com) but not auth error
    assert_ne!(resp.status_code(), axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_invalid_bearer_token() {
    let server = test_app_with_auth(vec!["test-key-123".into()]);
    let resp = server
        .post("/v1/scrape")
        .add_header(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer wrong-key"),
        )
        .json(&json!({"url": "https://example.com"}))
        .await;
    resp.assert_status(axum::http::StatusCode::UNAUTHORIZED);
    let json: serde_json::Value = resp.json();
    assert_eq!(json["success"], false);
}

#[tokio::test]
async fn auth_missing_header() {
    let server = test_app_with_auth(vec!["test-key-123".into()]);
    let resp = server
        .post("/v1/scrape")
        .json(&json!({"url": "https://example.com"}))
        .await;
    resp.assert_status(axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_no_keys_configured_allows_all() {
    let server = test_app_no_auth();
    let resp = server
        .post("/v1/scrape")
        .json(&json!({"url": "https://example.com"}))
        .await;
    // Should not be 401
    assert_ne!(resp.status_code(), axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_basic_scheme_rejected() {
    let server = test_app_with_auth(vec!["test-key-123".into()]);
    let resp = server
        .post("/v1/scrape")
        .add_header(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Basic dGVzdC1rZXktMTIz"),
        )
        .json(&json!({"url": "https://example.com"}))
        .await;
    resp.assert_status(axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_bearer_case_sensitive() {
    let server = test_app_with_auth(vec!["test-key-123".into()]);
    // "bearer" (lowercase) should be rejected — code checks for "Bearer " (capital B)
    let resp = server
        .post("/v1/scrape")
        .add_header(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("bearer test-key-123"),
        )
        .json(&json!({"url": "https://example.com"}))
        .await;
    resp.assert_status(axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_empty_token() {
    let server = test_app_with_auth(vec!["test-key-123".into()]);
    let resp = server
        .post("/v1/scrape")
        .add_header(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer "),
        )
        .json(&json!({"url": "https://example.com"}))
        .await;
    resp.assert_status(axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_very_long_token() {
    let server = test_app_with_auth(vec!["test-key-123".into()]);
    let long_token = "x".repeat(10000);
    let header_val = HeaderValue::from_str(&format!("Bearer {long_token}")).unwrap();
    let resp = server
        .post("/v1/scrape")
        .add_header(axum::http::header::AUTHORIZATION, header_val)
        .json(&json!({"url": "https://example.com"}))
        .await;
    resp.assert_status(axum::http::StatusCode::UNAUTHORIZED);
}

// ── Constant-time comparison tests ──

#[test]
fn constant_time_eq_identical() {
    assert!(crw_server::middleware::constant_time_eq_pub(
        b"hello", b"hello"
    ));
}

#[test]
fn constant_time_eq_same_length() {
    assert!(!crw_server::middleware::constant_time_eq_pub(
        b"hello", b"world"
    ));
}

#[test]
fn constant_time_eq_different_length() {
    assert!(!crw_server::middleware::constant_time_eq_pub(
        b"hello", b"hi"
    ));
}

#[test]
fn constant_time_eq_empty() {
    assert!(crw_server::middleware::constant_time_eq_pub(b"", b""));
}
