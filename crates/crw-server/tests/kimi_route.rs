//! Integration tests for the `/kimi/*` compatibility surface (Kimi Code's
//! `moonshot_search` + `moonshot_fetch` tools).
//!
//! `search` is exercised against a `wiremock` SearXNG (like `search_route.rs`);
//! `fetch` is exercised against a `wiremock` HTTP page over the HTTP-only
//! renderer, gated by the `CRW_ALLOW_LOOPBACK_FOR_TESTS` opt-in so SSRF
//! validation permits the loopback mock (same pattern as `map_filter.rs`).

use axum_test::TestServer;
use crw_core::config::AppConfig;
use crw_server::app::create_app;
use crw_server::state::AppState;
use serde_json::{Value, json};
use std::sync::Once;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

static INIT_TEST_ENV: Once = Once::new();
fn allow_loopback_for_tests() {
    INIT_TEST_ENV.call_once(|| {
        // SAFETY: set once before any tokio worker spawns its own thread.
        unsafe {
            std::env::set_var("CRW_ALLOW_LOOPBACK_FOR_TESTS", "1");
        }
    });
}

fn test_app_with_searxng(url: &str) -> TestServer {
    let toml = format!(
        r#"
[search]
enabled = true
searxng_url = "{url}"
timeout_ms = 5000
"#
    );
    let config: AppConfig = toml::from_str(&toml).unwrap();
    let state = AppState::new(config).expect("AppState::new failed");
    TestServer::new(create_app(state))
}

fn test_app_with_auth_and_searxng(url: &str) -> TestServer {
    let toml = format!(
        r#"
[auth]
api_keys = ["secret-key"]

[search]
enabled = true
searxng_url = "{url}"
timeout_ms = 5000
"#
    );
    let config: AppConfig = toml::from_str(&toml).unwrap();
    let state = AppState::new(config).expect("AppState::new failed");
    TestServer::new(create_app(state))
}

fn test_app_plain() -> TestServer {
    let config: AppConfig = toml::from_str("").unwrap();
    let state = AppState::new(config).expect("AppState::new failed");
    TestServer::new(create_app(state))
}

/// Two results; the higher-scoring one uses a `www.` host so the top result
/// (surviving `limit: 1`) exercises the `site_name` www-strip.
fn searxng_response() -> Value {
    json!({
        "query": "rust async",
        "number_of_results": 2,
        "results": [
            {
                "url": "https://www.tokio.rs/",
                "title": "Tokio",
                "engine": "google",
                "content": "An asynchronous runtime for Rust",
                "score": 2.0,
                "category": "general",
                "template": "default.html"
            },
            {
                "url": "https://docs.rs/async-std/",
                "title": "async-std",
                "engine": "duckduckgo",
                "content": "An async standard library",
                "score": 1.0,
                "category": "general",
                "template": "default.html"
            }
        ],
        "answers": [],
        "corrections": [],
        "infoboxes": [],
        "suggestions": [],
        "unresponsive_engines": []
    })
}

const FETCH_HTML: &str = r#"<!doctype html>
<html><head><title>Kimi Fetch Fixture</title></head>
<body><main><h1>Hello Kimi</h1>
<p>This page is scraped by the kimi fetch adapter into markdown.</p></main></body></html>"#;

#[tokio::test]
async fn kimi_search_shapes_results_and_honors_limit() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(searxng_response()))
        .mount(&mock)
        .await;

    let server = test_app_with_searxng(&mock.uri());
    // Unknown fields (`enable_page_crawling`, `timeout_seconds`) must be tolerated.
    let resp = server
        .post("/kimi/search")
        .json(&json!({
            "text_query": "rust async",
            "limit": 1,
            "enable_page_crawling": true,
            "timeout_seconds": 5
        }))
        .await;
    resp.assert_status_ok();

    let body: Value = resp.json();
    let results = body["search_results"]
        .as_array()
        .expect("search_results should be an array");
    assert_eq!(results.len(), 1, "limit=1 must be honored");
    assert_eq!(results[0]["url"], "https://www.tokio.rs/");
    assert_eq!(results[0]["site_name"], "tokio.rs", "www. must be stripped");
    assert_eq!(results[0]["snippet"], "An asynchronous runtime for Rust");
    assert_eq!(results[0]["title"], "Tokio");
}

#[tokio::test]
async fn kimi_search_requires_auth_when_configured() {
    let mock = MockServer::start().await;
    let server = test_app_with_auth_and_searxng(&mock.uri());
    let resp = server
        .post("/kimi/search")
        .json(&json!({"text_query": "rust async"}))
        .await;
    resp.assert_status(axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn kimi_fetch_returns_markdown() {
    allow_loopback_for_tests();
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html; charset=utf-8")
                .set_body_string(FETCH_HTML),
        )
        .mount(&mock)
        .await;

    let server = test_app_plain();
    let resp = server
        .post("/kimi/fetch")
        .json(&json!({"url": format!("{}/", mock.uri())}))
        .await;
    resp.assert_status_ok();

    let content_type = resp.header("content-type").to_str().unwrap().to_string();
    assert!(
        content_type.starts_with("text/markdown"),
        "expected text/markdown, got: {content_type}"
    );
    let text = resp.text();
    assert!(text.contains("Hello Kimi"), "markdown body was: {text}");
}

#[tokio::test]
async fn kimi_fetch_non200_target_is_error() {
    allow_loopback_for_tests();
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(404)
                .insert_header("content-type", "text/html; charset=utf-8")
                .set_body_string("<html><body>not found</body></html>"),
        )
        .mount(&mock)
        .await;

    let server = test_app_plain();
    let resp = server
        .post("/kimi/fetch")
        .json(&json!({"url": format!("{}/", mock.uri())}))
        .await;
    assert_ne!(
        resp.status_code(),
        axum::http::StatusCode::OK,
        "a blocked/404 fetch must be a non-200 error for Kimi"
    );
}
