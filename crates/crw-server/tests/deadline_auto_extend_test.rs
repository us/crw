//! Integration tests for issue #35: verify that
//! `request.auto_extend_deadline_for_ladder` lets the configured CDP tier
//! timeouts (e.g. `chrome_timeout_ms = 30000`) actually run to completion
//! instead of being silently clamped by a small `deadline_ms_default`.
//!
//! These tests require a live Chrome CDP endpoint and are gated behind
//! `CRW_CDP_WS_URL` + `#[ignore]`. They do not run in the default
//! `cargo test --workspace` invocation. Run them locally with:
//!
//! ```sh
//! CRW_CDP_WS_URL="ws://localhost:9222/devtools/browser/<id>" \
//!     cargo test -p crw-server --features cdp --test deadline_auto_extend_test \
//!     -- --ignored
//! ```
//!
//! The marker HTML is constructed so that the literal string
//! `"CHROME_RENDERED_OK"` only appears in the rendered DOM after Chrome
//! executes the inline script — never in the raw HTML bytes that an
//! HTTP-only fetch would receive. This eliminates false positives where
//! HTTP succeeded without the JS path being exercised.

#![cfg(feature = "cdp")]

use axum_test::TestServer;
use crw_core::config::{AppConfig, CdpEndpoint, RendererMode};
use crw_server::app::create_app;
use crw_server::state::AppState;
use serde_json::json;
use std::time::Duration;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Marker HTML: `loading` placeholder, replaced post-DOMContentLoaded by
/// the runtime concatenation of `'CHROME_'` + `'RENDERED_OK'`. The full
/// marker string never appears in the source.
const MARKER_HTML: &str = r#"<!doctype html>
<html><body>
<div id="content">loading</div>
<script>
  document.addEventListener('DOMContentLoaded', function () {
    document.getElementById('content').textContent = 'CHROME_' + 'RENDERED_OK';
  });
</script>
</body></html>"#;

const FULL_MARKER: &str = "CHROME_RENDERED_OK";

fn cdp_ws_url() -> Option<String> {
    std::env::var("CRW_CDP_WS_URL").ok()
}

fn chrome_only_config(ws_url: &str, deadline_ms_default: u64, auto_extend: bool) -> AppConfig {
    let mut cfg = AppConfig::default();
    cfg.request.deadline_ms_default = deadline_ms_default;
    cfg.request.auto_extend_deadline_for_ladder = auto_extend;
    cfg.renderer.mode = RendererMode::Chrome;
    cfg.renderer.page_timeout_ms = 30_000;
    cfg.renderer.chrome_timeout_ms = Some(30_000);
    cfg.renderer.chrome = Some(CdpEndpoint {
        ws_url: ws_url.to_string(),
    });
    cfg.renderer.lightpanda = None;
    cfg.renderer.playwright = None;
    cfg.server.request_timeout_secs = 60;
    cfg
}

async fn slow_marker_endpoint(delay: Duration) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(MARKER_HTML)
                .insert_header("content-type", "text/html; charset=utf-8")
                .set_delay(delay),
        )
        .mount(&server)
        .await;
    server
}

#[tokio::test]
#[ignore = "requires running Chrome CDP at CRW_CDP_WS_URL"]
async fn auto_extend_honors_chrome_timeout() {
    let Some(ws_url) = cdp_ws_url() else {
        return;
    };
    // 12s endpoint is unambiguously above the 8s strict deadline and below
    // the 30s chrome_timeout_ms — the test fails deterministically without
    // auto-extend, succeeds deterministically with it.
    let mock = slow_marker_endpoint(Duration::from_secs(12)).await;
    let cfg = chrome_only_config(&ws_url, 8_000, /*auto_extend=*/ true);
    let state = AppState::new(cfg).expect("AppState");
    let server = TestServer::new(create_app(state));

    let resp = server
        .post("/v1/scrape")
        .json(&json!({
            "url": format!("{}/", mock.uri()),
            "renderJs": true,
            "formats": ["markdown"],
        }))
        .await;
    resp.assert_status_ok();
    let body: serde_json::Value = resp.json();
    assert_eq!(
        body["success"], true,
        "auto-extend should permit chrome to finish: {body:?}"
    );
    let markdown = body["data"]["markdown"].as_str().unwrap_or_default();
    assert!(
        markdown.contains(FULL_MARKER),
        "rendered markdown must contain the JS-assembled marker; got: {markdown}"
    );
}

#[tokio::test]
#[ignore = "requires running Chrome CDP at CRW_CDP_WS_URL"]
async fn auto_extend_disabled_strict_deadline_times_out() {
    let Some(ws_url) = cdp_ws_url() else {
        return;
    };
    let mock = slow_marker_endpoint(Duration::from_secs(12)).await;
    let cfg = chrome_only_config(&ws_url, 8_000, /*auto_extend=*/ false);
    let state = AppState::new(cfg).expect("AppState");
    let server = TestServer::new(create_app(state));

    let resp = server
        .post("/v1/scrape")
        .json(&json!({
            "url": format!("{}/", mock.uri()),
            "renderJs": true,
            "formats": ["markdown"],
        }))
        .await;
    // Handler-side timeout, not Tower outer cancellation. CrwError::Timeout
    // serializes to errorCode = "timeout".
    let body: serde_json::Value = resp.json();
    assert_eq!(
        body["errorCode"], "timeout",
        "strict deadline should produce handler-side timeout, not tower cancel: {body:?}"
    );
}

#[tokio::test]
#[ignore = "requires running Chrome CDP at CRW_CDP_WS_URL"]
async fn explicit_deadline_bypasses_auto_extend() {
    let Some(ws_url) = cdp_ws_url() else {
        return;
    };
    let mock = slow_marker_endpoint(Duration::from_secs(12)).await;
    let cfg = chrome_only_config(&ws_url, 30_000, /*auto_extend=*/ true);
    let state = AppState::new(cfg).expect("AppState");
    let server = TestServer::new(create_app(state));

    // Caller's explicit deadline beats both default and ladder_min.
    let resp = server
        .post("/v1/scrape")
        .json(&json!({
            "url": format!("{}/", mock.uri()),
            "renderJs": true,
            "formats": ["markdown"],
            "deadlineMs": 3_000,
        }))
        .await;
    let body: serde_json::Value = resp.json();
    assert_eq!(
        body["errorCode"], "timeout",
        "explicit 3s deadline should cut the 12s response: {body:?}"
    );
}
