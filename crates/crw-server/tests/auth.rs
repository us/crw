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

// ── Ops/admin routes are behind the auth boundary ──
//
// Regression guard for the pre-fix state where `/metrics`,
// `/metrics/renderer-breakers`, and `/admin/breakers/reset` were mounted on the
// base router OUTSIDE `auth_middleware` and were reachable with no credential
// even on a key-secured deployment.

#[tokio::test]
async fn metrics_requires_auth_when_keys_set() {
    let server = test_app_with_auth(vec!["ops-key".into()]);
    server
        .get("/metrics")
        .await
        .assert_status(axum::http::StatusCode::UNAUTHORIZED);
    server
        .get("/metrics")
        .add_header(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer ops-key"),
        )
        .await
        .assert_status_ok();
}

#[tokio::test]
async fn renderer_breakers_requires_auth_when_keys_set() {
    let server = test_app_with_auth(vec!["ops-key".into()]);
    server
        .get("/metrics/renderer-breakers")
        .await
        .assert_status(axum::http::StatusCode::UNAUTHORIZED);
    server
        .get("/metrics/renderer-breakers")
        .add_header(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer ops-key"),
        )
        .await
        .assert_status_ok();
}

#[tokio::test]
async fn admin_reset_requires_auth_when_keys_set() {
    let server = test_app_with_auth(vec!["ops-key".into()]);
    // No credential → 401, and the breaker reset never runs.
    server
        .post("/admin/breakers/reset")
        .await
        .assert_status(axum::http::StatusCode::UNAUTHORIZED);
    // Valid key → the admin action runs.
    let resp = server
        .post("/admin/breakers/reset")
        .add_header(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer ops-key"),
        )
        .await;
    resp.assert_status_ok();
    let json: serde_json::Value = resp.json();
    assert_eq!(json["ok"], true);
}

#[tokio::test]
async fn ops_routes_open_when_no_keys() {
    // Back-compat: with no keys configured the ops routes stay reachable,
    // exactly like the scraper API (default self-host behavior).
    let server = test_app_no_auth();
    server.get("/metrics").await.assert_status_ok();
    server
        .get("/metrics/renderer-breakers")
        .await
        .assert_status_ok();
    server
        .post("/admin/breakers/reset")
        .await
        .assert_status_ok();
}

#[tokio::test]
async fn liveness_and_schema_never_require_auth() {
    // Health/ready/openapi must stay open even with keys set — they are how
    // load balancers and SDK generators reach the service.
    let server = test_app_with_auth(vec!["ops-key".into()]);
    server.get("/health").await.assert_status_ok();
    server.get("/ready").await.assert_status_ok();
    server.get("/openapi.json").await.assert_status_ok();
    server.get("/openapi-3.0.json").await.assert_status_ok();
}

// ── CORS defaults ──

fn test_app_with_cors(origins: Vec<String>) -> TestServer {
    let toml_str = format!(
        r#"
[server]
cors_allowed_origins = {:?}
"#,
        origins
    );
    let config: AppConfig = toml::from_str(&toml_str).unwrap();
    let state = AppState::new(config).expect("AppState::new failed");
    TestServer::new(create_app(state))
}

#[tokio::test]
async fn cors_absent_by_default() {
    // Default config emits NO Access-Control-Allow-Origin on API/ops routes —
    // browsers block cross-origin reads. (The old permissive layer set `*`.)
    let server = test_app_no_auth();
    let resp = server
        .get("/metrics")
        .add_header(
            axum::http::header::ORIGIN,
            HeaderValue::from_static("https://evil.example"),
        )
        .await;
    assert!(
        resp.headers()
            .get(axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none(),
        "default config must not emit ACAO on /metrics"
    );
}

#[tokio::test]
async fn cors_allowlist_echoes_configured_origin() {
    let server = test_app_with_cors(vec!["https://app.example.com".into()]);
    let resp = server
        .get("/health")
        .add_header(
            axum::http::header::ORIGIN,
            HeaderValue::from_static("https://app.example.com"),
        )
        .await;
    assert_eq!(
        resp.headers()
            .get(axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .and_then(|v| v.to_str().ok()),
        Some("https://app.example.com"),
    );
}

#[tokio::test]
async fn cors_allowlist_rejects_unlisted_origin() {
    // The property that actually makes it an allowlist: an origin NOT on the
    // list is not echoed. Guards against a refactor to a mirror/permissive
    // layer, which `cors_allowlist_echoes_configured_origin` alone would miss.
    let server = test_app_with_cors(vec!["https://app.example.com".into()]);
    let resp = server
        .get("/health")
        .add_header(
            axum::http::header::ORIGIN,
            HeaderValue::from_static("https://evil.example"),
        )
        .await;
    // Assert ACAO is absent (not merely "not the evil origin") — the strong
    // guard: a mirror layer OR the old `permissive()` (`ACAO: *`) would both
    // fail this, so it locks in true allowlist behavior.
    assert!(
        resp.headers()
            .get(axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none(),
        "an unlisted origin must not receive any Access-Control-Allow-Origin"
    );
}

#[tokio::test]
async fn cors_null_origin_entry_is_rejected() {
    // `null` is the opaque-origin token (sandboxed iframes, file://). It must be
    // dropped like `*`, never turned into an allowed origin.
    let server = test_app_with_cors(vec!["null".into()]);
    let resp = server
        .get("/health")
        .add_header(axum::http::header::ORIGIN, HeaderValue::from_static("null"))
        .await;
    assert!(
        resp.headers()
            .get(axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none(),
        "`null` entry must not produce an allowed opaque origin"
    );
}

#[tokio::test]
async fn cors_wildcard_entry_is_rejected_not_panicking() {
    // A literal "*" must be dropped (never re-enable wildcard CORS) and must not
    // panic at startup — `AllowOrigin::list(["*"])` would panic in tower-http.
    let server = test_app_with_cors(vec!["*".into()]);
    let resp = server
        .get("/health")
        .add_header(
            axum::http::header::ORIGIN,
            HeaderValue::from_static("https://evil.example"),
        )
        .await;
    assert!(
        resp.headers()
            .get(axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none(),
        "`*` entry must not produce a wildcard ACAO"
    );
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
