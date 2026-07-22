//! `/v1/search` + `scrapeOptions` per-result scrape budget.
//!
//! The enrichment fan-out used to hand every result the IMPLICIT request
//! deadline, which auto-extends to the whole renderer ladder (92.5s on the
//! docker renderer config). Search waits for every result, so one straggler
//! walking that ladder stalled the entire response. The budget is now bounded
//! and caller-overridable via `scrapeOptions.timeout`.
//!
//! These tests pin the HTTP-only tier deliberately: it is the only tier where
//! an elapsed budget surfaces as an `Err` (its reqwest client is built with
//! `.timeout(deadline.remaining())`), so "no markdown + `error` set" is the
//! guaranteed outcome. A CDP tier would instead return `Ok` with a partial DOM.

use axum_test::TestServer;
use crw_core::config::AppConfig;
use crw_server::app::create_app;
use crw_server::state::AppState;
use serde_json::{Value, json};
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The enrichment scrape resolves the result URL, so loopback has to be allowed.
fn allow_loopback() {
    // SAFETY: test-only env opt-in, set before the server handles any request.
    unsafe {
        std::env::set_var("CRW_ALLOW_LOOPBACK_FOR_TESTS", "1");
    }
}

fn test_app(searxng_url: &str) -> TestServer {
    let toml = format!(
        r#"
[search]
enabled = true
searxng_url = "{searxng_url}"
timeout_ms = 5000

[request]
# Prod-shaped: an implicit deadline would be 60s, far past any of the budgets
# asserted below — so a regression to the old behaviour fails these tests.
deadline_ms_default = 60000
"#
    );
    let config: AppConfig = toml::from_str(&toml).unwrap();
    let state = AppState::new(config).expect("AppState::new failed");
    TestServer::new(create_app(state))
}

/// SearXNG stub returning a single result that points back at `page_url`.
async fn mock_searxng_and_page(page_delay: Duration) -> MockServer {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/slow"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(page_delay)
                .set_body_string("<html><body><article>hello from the page</article></body></html>")
                .insert_header("content-type", "text/html"),
        )
        .mount(&mock)
        .await;
    let page_url = format!("{}/slow", mock.uri());
    Mock::given(method("GET"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "query": "q",
            "number_of_results": 1,
            "results": [{
                "url": page_url,
                "title": "slow page",
                "engine": "google",
                "content": "snippet",
                "score": 1.0,
                "category": "general",
                "template": "default.html"
            }],
            "answers": [], "corrections": [], "infoboxes": [],
            "suggestions": [], "unresponsive_engines": []
        })))
        .mount(&mock)
        .await;
    mock
}

#[tokio::test]
async fn enrichment_scrape_is_cut_at_the_caller_budget() {
    allow_loopback();
    let mock = mock_searxng_and_page(Duration::from_millis(2_000)).await;
    let server = test_app(&mock.uri());

    let started = std::time::Instant::now();
    let resp = server
        .post("/v1/search")
        .json(&json!({
            "query": "q",
            "limit": 1,
            "scrapeOptions": {"formats": ["markdown"], "timeout": 300}
        }))
        .await;
    let elapsed = started.elapsed();
    resp.assert_status_ok();

    let body: Value = resp.json();
    let results = body["data"]["results"].as_array().expect("flat results");
    assert_eq!(results.len(), 1);
    // Budget elapsed → no content, and the failure is visible on the result
    // rather than looking like a page that simply had no markdown.
    assert!(results[0]["markdown"].is_null(), "{:?}", results[0]);
    assert!(!results[0]["error"].is_null(), "{:?}", results[0]);
    // The snippet survives, so the result itself is not lost.
    assert_eq!(results[0]["description"], "snippet");
    assert!(
        elapsed < Duration::from_millis(2_000),
        "search waited for the whole page instead of its budget: {elapsed:?}"
    );
}

#[tokio::test]
async fn caller_timeout_raises_the_budget() {
    allow_loopback();
    let mock = mock_searxng_and_page(Duration::from_millis(300)).await;
    let server = test_app(&mock.uri());

    let resp = server
        .post("/v1/search")
        .json(&json!({
            "query": "q",
            "limit": 1,
            "scrapeOptions": {"formats": ["markdown"], "timeout": 10_000}
        }))
        .await;
    resp.assert_status_ok();

    let body: Value = resp.json();
    let results = body["data"]["results"].as_array().expect("flat results");
    let markdown = results[0]["markdown"].as_str().unwrap_or_default();
    assert!(
        markdown.contains("hello from the page"),
        "expected scraped content, got {:?}",
        results[0]
    );
    assert!(results[0]["truncated"].is_null(), "{:?}", results[0]);
}

#[tokio::test]
async fn out_of_range_timeout_is_rejected() {
    let mock = MockServer::start().await;
    let server = test_app(&mock.uri());
    for bad in [0u64, 60_001] {
        let resp = server
            .post("/v1/search")
            .json(&json!({
                "query": "q",
                "scrapeOptions": {"formats": ["markdown"], "timeout": bad}
            }))
            .await;
        assert_eq!(
            resp.status_code(),
            400,
            "timeout {bad} should be rejected, got {}",
            resp.text()
        );
    }
}
