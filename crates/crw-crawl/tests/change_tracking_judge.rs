//! Wiremock-backed tests for the one-shot scrape `changeTracking + goal +
//! judgeEnabled` path (`crw_crawl::single::scrape_url`).
//!
//! This is the stateless change-detection primitive the open core keeps: a
//! single scrape diffed against a caller-supplied snapshot, with an optional
//! LLM judge. Both the page origin and the OpenAI-compatible provider are
//! mocked, so the test runs offline and never touches a browser.

use std::sync::Arc;

use crw_core::config::{ExtractionConfig, LlmConfig, RendererConfig, StealthConfig};
use crw_core::types::{
    ChangeStatus, ChangeTrackingMode, ChangeTrackingOptions, ChangeTrackingSnapshot, OutputFormat,
    ScrapeRequest,
};
use crw_crawl::single::scrape_url;
use crw_renderer::FallbackRenderer;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const UA: &str = "crw-test/0.0";

/// OpenAI-compatible provider pointed at a mock server.
fn mock_llm(base_url: String) -> LlmConfig {
    LlmConfig {
        provider: "openai".into(),
        api_key: "test-key".into(),
        model: "gpt-4o-mini".into(),
        base_url: Some(base_url),
        ..Default::default()
    }
}

/// Tool-call envelope the judge parses.
fn judge_response(meaningful: bool, reason: &str) -> serde_json::Value {
    json!({
        "choices": [{
            "message": {
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {
                        "name": "judge_change",
                        "arguments": json!({
                            "meaningful": meaningful,
                            "confidence": "high",
                            "reason": reason,
                        }).to_string()
                    }
                }]
            }
        }],
        "usage": { "prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120 }
    })
}

/// HTTP-only renderer (no JS, no browser) — enough to fetch a mocked origin.
fn http_renderer() -> Arc<FallbackRenderer> {
    let cfg = RendererConfig::default();
    let stealth = StealthConfig::default();
    Arc::new(FallbackRenderer::new(&cfg, UA, None, &stealth).expect("renderer"))
}

/// Serve `body` as an HTML page and return its URL.
async fn page_server(body: &str) -> (MockServer, String) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/page"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html; charset=utf-8")
                .set_body_string(body.to_string()),
        )
        .mount(&server)
        .await;
    let url = format!("{}/page", server.uri());
    (server, url)
}

/// A scrape request that change-tracks `url` against `previous_markdown`.
fn ct_request(url: &str, previous_markdown: &str) -> ScrapeRequest {
    ScrapeRequest {
        url: url.to_string(),
        formats: vec![OutputFormat::Markdown, OutputFormat::ChangeTracking],
        change_tracking: Some(ChangeTrackingOptions {
            modes: vec![ChangeTrackingMode::GitDiff],
            previous: Some(ChangeTrackingSnapshot {
                markdown: Some(previous_markdown.to_string()),
                content_hash: crw_diff::snapshot::hash_markdown(previous_markdown),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
}

async fn run(req: &ScrapeRequest, llm: Option<&LlmConfig>) -> crw_core::types::ScrapeData {
    scrape_url(
        req,
        &http_renderer(),
        llm,
        &ExtractionConfig::default(),
        UA,
        false,
        Some(false),
        crw_core::deadline::Deadline::from_request_ms(30_000),
    )
    .await
    .expect("scrape")
}

/// The kept path end to end: a changed page + goal + judgeEnabled reaches the
/// judge, and the judgment lands on the change-tracking result.
#[tokio::test]
async fn scrape_change_tracking_runs_judge_against_mocked_llm() {
    let (_page, url) =
        page_server("<html><body><h1>Pro plan costs 99 USD</h1></body></html>").await;

    let llm_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(judge_response(true, "The plan price changed.")),
        )
        .expect(1) // the judge is actually called exactly once
        .mount(&llm_server)
        .await;

    let mut req = ct_request(&url, "# Pro plan costs 49 USD");
    req.goal = Some("Alert me when pricing changes".into());
    req.judge_enabled = Some(true);

    let llm = mock_llm(format!("{}/v1", llm_server.uri()));
    let data = run(&req, Some(&llm)).await;

    let ct = data.change_tracking.expect("change_tracking result");
    assert_eq!(ct.status, ChangeStatus::Changed);
    assert!(ct.diff.is_some(), "a changed page must produce a diff");

    let judgment = ct
        .judgment
        .unwrap_or_else(|| panic!("judge must run; warnings={:?}", data.warnings));
    assert!(judgment.meaningful);
    assert_eq!(judgment.reason, "The plan price changed.");
    assert!(data.warnings.is_empty(), "warnings: {:?}", data.warnings);
    // `.expect(1)` above is asserted on drop.
}

/// The judge is opt-in: without `goal` + `judgeEnabled`, change tracking stays
/// a deterministic, LLM-free diff. This is the open-core product boundary.
#[tokio::test]
async fn scrape_change_tracking_is_pure_diff_without_judge() {
    let (_page, url) =
        page_server("<html><body><h1>Pro plan costs 99 USD</h1></body></html>").await;

    // Any call to this server is a failure: no route is mounted, so a request
    // would 404 and the judge would surface a warning.
    let llm_server = MockServer::start().await;
    let llm = mock_llm(format!("{}/v1", llm_server.uri()));

    let req = ct_request(&url, "# Pro plan costs 49 USD");
    let data = run(&req, Some(&llm)).await;

    let ct = data.change_tracking.expect("change_tracking result");
    assert_eq!(ct.status, ChangeStatus::Changed);
    assert!(ct.diff.is_some());
    assert!(
        ct.judgment.is_none(),
        "no judge without goal + judgeEnabled"
    );
    assert!(data.warnings.is_empty(), "warnings: {:?}", data.warnings);
    assert!(
        llm_server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty(),
        "pure diff must not call an LLM"
    );
}

/// A `goal` alone must NOT trigger the judge: `judgeEnabled` is the opt-in.
/// Guards a paid path — if the gate ever relaxed to fire on a non-empty `goal`,
/// every scrape carrying one would silently make an LLM call and bill for it.
#[tokio::test]
async fn scrape_change_tracking_goal_alone_does_not_trigger_judge() {
    let (_page, url) =
        page_server("<html><body><h1>Pro plan costs 99 USD</h1></body></html>").await;

    // No route mounted: any judge call would 404 and surface a warning.
    let llm_server = MockServer::start().await;
    let llm = mock_llm(format!("{}/v1", llm_server.uri()));

    let mut req = ct_request(&url, "# Pro plan costs 49 USD");
    req.goal = Some("Alert me when pricing changes".into());
    req.judge_enabled = None; // opt-in absent

    let data = run(&req, Some(&llm)).await;

    let ct = data.change_tracking.expect("change_tracking result");
    assert_eq!(ct.status, ChangeStatus::Changed);
    assert!(
        ct.judgment.is_none(),
        "goal without judgeEnabled must not run the judge"
    );
    assert!(
        llm_server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty(),
        "goal alone must not call an LLM"
    );
    assert!(data.warnings.is_empty(), "warnings: {:?}", data.warnings);
}

/// `judgeEnabled` with no LLM configured degrades to a warning, never an error.
#[tokio::test]
async fn scrape_change_tracking_judge_skipped_without_llm() {
    let (_page, url) =
        page_server("<html><body><h1>Pro plan costs 99 USD</h1></body></html>").await;

    let mut req = ct_request(&url, "# Pro plan costs 49 USD");
    req.goal = Some("Alert me when pricing changes".into());
    req.judge_enabled = Some(true);

    let data = run(&req, None).await;

    let ct = data.change_tracking.expect("change_tracking result");
    assert_eq!(ct.status, ChangeStatus::Changed);
    assert!(ct.judgment.is_none());
    assert!(
        data.warnings.iter().any(|w| w.contains("judge skipped")),
        "expected a judge-skipped warning, got {:?}",
        data.warnings
    );
}
