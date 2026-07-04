//! Integration tests for `POST /v1/search`.
//!
//! Uses `wiremock` to stand up a fake SearXNG that the route's
//! `SearxngClient` talks to via the configured `searxng_url`. We do NOT
//! exercise scrape enrichment here (it would need real network access);
//! `crw-search` unit tests cover param mapping and result transforms.

use axum_test::TestServer;
use crw_core::config::AppConfig;
use crw_server::app::create_app;
use crw_server::state::AppState;
use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
    let app = create_app(state);
    TestServer::new(app)
}

fn test_app_with_searxng_and_llm(url: &str) -> TestServer {
    // Sibling of `test_app_with_searxng` that also wires an `[extraction.llm]`
    // leg so provider/model can fall back to server config. `api_key` has no
    // serde default (required field) — must be present for the block to parse.
    let toml = format!(
        r#"
[search]
enabled = true
searxng_url = "{url}"
timeout_ms = 5000

[extraction.llm]
provider = "anthropic"
api_key = "sk-test"
model = "claude-sonnet-4-20250514"
"#
    );
    let config: AppConfig = toml::from_str(&toml).unwrap();
    let state = AppState::new(config).expect("AppState::new failed");
    let app = create_app(state);
    TestServer::new(app)
}

fn test_app_search_disabled() -> TestServer {
    // Default config has no searxng_url → state.searxng = None.
    let config: AppConfig = toml::from_str("").unwrap();
    let state = AppState::new(config).expect("AppState::new failed");
    let app = create_app(state);
    TestServer::new(app)
}

fn searxng_general_response() -> Value {
    json!({
        "query": "rust async",
        "number_of_results": 2,
        "results": [
            {
                "url": "https://tokio.rs/",
                "title": "Tokio — async runtime",
                "engine": "google",
                "content": "An asynchronous runtime for Rust",
                "score": 1.5,
                "category": "general",
                "template": "default.html"
            },
            {
                "url": "https://docs.rs/async-std/",
                "title": "async-std",
                "engine": "duckduckgo",
                "content": "An async standard library",
                "score": 1.2,
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

fn searxng_mixed_response() -> Value {
    json!({
        "query": "rust",
        "number_of_results": 3,
        "results": [
            {
                "url": "https://news.example.com/rust",
                "title": "Rust 2026 release",
                "engine": "bing news",
                "content": "Rust gets new features",
                "score": 1.0,
                "category": "news",
                "template": "default.html",
                "publishedDate": "2026-05-01T00:00:00Z"
            },
            {
                "url": "https://rust-lang.org/",
                "title": "The Rust Programming Language",
                "engine": "google",
                "content": "Empowering everyone",
                "score": 1.5,
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

#[tokio::test]
async fn search_llm_usage_always_present_on_zero_results() {
    // Wave 4 (R1) invariant: once LLM mode is entered (summarizeResults +
    // scrapeOptions present), `/v1/search` ALWAYS returns a non-null
    // `llmUsage` object — even with ZERO search results and no LLM call
    // actually running. The SaaS 5-branch credit dispatch relies on this.
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"results": [], "number_of_results": 0})),
        )
        .mount(&mock)
        .await;

    let server = test_app_with_searxng_and_llm(&mock.uri());
    let resp = server
        .post("/v1/search")
        .json(&json!({
            "query": "rust async",
            "summarizeResults": true,
            "scrapeOptions": {"formats": ["markdown"]}
        }))
        .await;
    resp.assert_status_ok();
    let body: Value = resp.json();
    assert_eq!(body["success"], true);

    let usage = &body["data"]["llmUsage"];
    assert!(usage.is_object(), "llmUsage must be present, got: {usage}");
    assert_eq!(usage["executedSummaries"], 0);
    assert_eq!(usage["answerExecuted"], false);
    assert_eq!(usage["inputTokens"], 0);
    assert_eq!(usage["outputTokens"], 0);
    assert!(
        usage["provider"].is_string(),
        "provider must be a string, got: {}",
        usage["provider"]
    );
    assert!(
        usage["model"].is_string(),
        "model must be a string, got: {}",
        usage["model"]
    );
}

#[tokio::test]
async fn search_returns_flat_results() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("q", "rust async"))
        .and(query_param("format", "json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(searxng_general_response()))
        .mount(&mock)
        .await;

    let server = test_app_with_searxng(&mock.uri());
    let resp = server
        .post("/v1/search")
        .json(&json!({"query": "rust async", "limit": 5}))
        .await;
    resp.assert_status_ok();
    let body: Value = resp.json();
    assert_eq!(body["success"], true);
    let data = body["data"]["results"]
        .as_array()
        .expect("flat results should be array");
    assert_eq!(data.len(), 2);
    // Highest-score result first (1.5 > 1.2).
    assert_eq!(data[0]["url"], "https://tokio.rs/");
    assert_eq!(data[0]["position"], 1);
    assert_eq!(data[1]["position"], 2);
    assert_eq!(data[0]["description"], "An asynchronous runtime for Rust");
}

#[tokio::test]
async fn search_returns_grouped_when_sources_set() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(searxng_mixed_response()))
        .mount(&mock)
        .await;

    let server = test_app_with_searxng(&mock.uri());
    let resp = server
        .post("/v1/search")
        .json(&json!({"query": "rust", "sources": ["web", "news"]}))
        .await;
    resp.assert_status_ok();
    let body: Value = resp.json();
    let data = &body["data"]["results"];
    assert!(data["web"].is_array(), "expected grouped 'web' bucket");
    assert!(data["news"].is_array(), "expected grouped 'news' bucket");
    assert_eq!(data["web"][0]["url"], "https://rust-lang.org/");
    assert_eq!(data["news"][0]["url"], "https://news.example.com/rust");
}

#[tokio::test]
async fn search_disabled_returns_503_with_search_disabled_code() {
    // When `[search].searxng_url` is unset, the route returns 503 Service
    // Unavailable + `error_code: "search_disabled"` so callers can distinguish
    // "operator turned this off" from a generic 400 (which would suggest a
    // bad request body).
    let server = test_app_search_disabled();
    let resp = server
        .post("/v1/search")
        .json(&json!({"query": "anything"}))
        .await;
    resp.assert_status(axum::http::StatusCode::SERVICE_UNAVAILABLE);
    let body: Value = resp.json();
    assert_eq!(body["success"], false);
    assert_eq!(body["errorCode"], "search_disabled");
    let err = body["error"].as_str().unwrap();
    assert!(
        err.contains("Search is disabled"),
        "expected disabled error, got: {err}"
    );
}

#[tokio::test]
async fn search_rejects_empty_query() {
    let mock = MockServer::start().await;
    let server = test_app_with_searxng(&mock.uri());
    let resp = server.post("/v1/search").json(&json!({"query": ""})).await;
    resp.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn search_rejects_oversized_query() {
    let mock = MockServer::start().await;
    let server = test_app_with_searxng(&mock.uri());
    let q = "x".repeat(2001);
    let resp = server.post("/v1/search").json(&json!({"query": q})).await;
    resp.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn search_rejects_limit_above_max() {
    let mock = MockServer::start().await;
    let server = test_app_with_searxng(&mock.uri());
    let resp = server
        .post("/v1/search")
        .json(&json!({"query": "rust", "limit": 9999}))
        .await;
    resp.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn search_upstream_5xx_propagates() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(503).set_body_string("upstream down"))
        .mount(&mock)
        .await;

    let server = test_app_with_searxng(&mock.uri());
    let resp = server
        .post("/v1/search")
        .json(&json!({"query": "rust"}))
        .await;
    // Map: SearchError::Upstream → CrwError::HttpError → 502 Bad Gateway.
    let status = resp.status_code();
    assert!(
        status == axum::http::StatusCode::BAD_GATEWAY
            || status == axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        "expected 502 or 500 for upstream failure, got {status}"
    );
}

#[tokio::test]
async fn search_pdf_category_modifies_query() {
    let mock = MockServer::start().await;
    // Verify the upstream `q` carries the appended `filetype:pdf` operator.
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("q", "rust filetype:pdf"))
        .respond_with(ResponseTemplate::new(200).set_body_json(searxng_general_response()))
        .mount(&mock)
        .await;

    let server = test_app_with_searxng(&mock.uri());
    let resp = server
        .post("/v1/search")
        .json(&json!({"query": "rust", "categories": ["pdf"]}))
        .await;
    resp.assert_status_ok();
}

#[tokio::test]
async fn search_tbs_qdr_h_maps_to_time_range_day() {
    let mock = MockServer::start().await;
    // SaaS quirk: SearXNG has no hour granularity; `qdr:h` collapses to `day`.
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("time_range", "day"))
        .respond_with(ResponseTemplate::new(200).set_body_json(searxng_general_response()))
        .mount(&mock)
        .await;

    let server = test_app_with_searxng(&mock.uri());
    let resp = server
        .post("/v1/search")
        .json(&json!({"query": "rust", "tbs": "qdr:h"}))
        .await;
    resp.assert_status_ok();
}
