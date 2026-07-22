//! Integration coverage for `renderJs` on the v2 (Firecrawl-compat) surface.
//!
//! Regression for #346: the field had no home on `V2ScrapeRequest`, so the
//! lenient-unknown-fields policy swallowed it and every v2 request reached the
//! engine with `render_js: None`. Honoring it makes one previously-accepted
//! combination fail loudly, which is what this file pins down.
//!
//! Loopback is opted in via `CRW_ALLOW_LOOPBACK_FOR_TESTS` (same pattern as
//! `kimi_route.rs` / `map_filter.rs`) so SSRF validation lets a 127.0.0.1 target
//! through and the request reaches the handler. Nothing here performs a fetch:
//! the assertions all land on rejections raised before the renderer runs.

use axum_test::TestServer;
use crw_core::config::AppConfig;
use crw_server::app::create_app;
use crw_server::state::AppState;
use serde_json::{Value, json};
use std::sync::Once;

static INIT_TEST_ENV: Once = Once::new();
fn allow_loopback_for_tests() {
    INIT_TEST_ENV.call_once(|| {
        // SAFETY: set once before any tokio worker spawns its own thread.
        unsafe {
            std::env::set_var("CRW_ALLOW_LOOPBACK_FOR_TESTS", "1");
        }
    });
}

/// `mode = "none"` pins the renderer set to empty. Without it the default
/// `auto` mode would discover whatever browser happens to exist on the machine
/// running the tests, and `validate_renderer_pin` (which the crawl assertions
/// below hinge on) would then behave differently on a dev laptop with Chrome
/// installed than on CI.
fn test_app() -> TestServer {
    allow_loopback_for_tests();
    let config: AppConfig = toml::from_str("[renderer]\nmode = \"none\"\n").unwrap();
    let state = AppState::new(config).expect("AppState::new failed");
    TestServer::new(create_app(state))
}

/// `screenshot` needs a CDP capture, so the engine refuses to pair it with an
/// explicit `renderJs:false` rather than returning a document with a null
/// screenshot (`crw-crawl/src/single.rs`). That guard is shared with `/v1` but
/// was unreachable from v2 while the field was being dropped: the same body
/// used to return 200. Honoring `renderJs` turns it into a 400, which is a
/// deliberate, user-visible behavior change and belongs in the record.
#[tokio::test]
async fn v2_scrape_screenshot_with_render_js_false_is_rejected() {
    let server = test_app();
    let res = server
        .post("/v2/scrape")
        .json(&json!({
            "url": "http://127.0.0.1:9/never-fetched",
            "formats": ["screenshot"],
            "renderJs": false,
        }))
        .await;

    assert_eq!(res.status_code(), 400, "body: {}", res.text());
    let body: Value = res.json();
    let err = body["error"].as_str().unwrap_or_default();
    assert!(
        err.contains("screenshot") && err.contains("renderJs"),
        "the error must name both sides of the contradiction, got: {err}"
    );
}

/// The same pair on the canonical `/firecrawl/v2` alias. Both prefixes are
/// built from one `routes::v2::router()` call, so this guards against a future
/// fork of the handler rather than testing a second code path today.
#[tokio::test]
async fn firecrawl_v2_scrape_screenshot_with_render_js_false_is_rejected() {
    let server = test_app();
    let res = server
        .post("/firecrawl/v2/scrape")
        .json(&json!({
            "url": "http://127.0.0.1:9/never-fetched",
            "formats": ["screenshot"],
            "renderJs": false,
        }))
        .await;

    assert_eq!(res.status_code(), 400, "body: {}", res.text());
}

/// Omitting `renderJs` must not inherit the rejection above — a screenshot on
/// its own is a perfectly ordinary v2 request. Proves the new field only bites
/// when the caller actually sets it.
#[tokio::test]
async fn v2_scrape_screenshot_without_render_js_is_not_rejected_upfront() {
    let server = test_app();
    let res = server
        .post("/v2/scrape")
        .json(&json!({
            "url": "http://127.0.0.1:9/refused",
            "formats": ["screenshot"],
        }))
        .await;

    // Nothing rejects this up front, so it runs on to the fetch and dies on the
    // refused port. Asserting that exact outcome (rather than merely "not 400")
    // pins down WHERE it failed: a validation error here would mean the
    // contradiction guard fired without an explicit `false`.
    assert_eq!(res.status_code(), 422, "body: {}", res.text());
    let body: Value = res.json();
    assert_eq!(body["errorCode"], "target_unreachable", "body: {body}");
}

/// The crawl half of the fix. `scrapeOptions.renderJs` has to reach
/// `CrawlRequest.render_js`, not just the intermediate `ScrapeOpts` projection.
/// `validate_crawl_renderer` reads that exact field: an explicit `false` means
/// the request never touches a browser, so the availability check for a pinned
/// renderer is skipped. With a default config (no JS tier configured) that
/// difference is observable as 200 vs 400, which is what makes this a real
/// end-to-end assertion on the field rather than on the parser.
#[tokio::test]
async fn v2_crawl_render_js_false_reaches_the_crawl_request() {
    let server = test_app();
    let res = server
        .post("/v2/crawl")
        .json(&json!({
            "url": "http://127.0.0.1:9/never-fetched",
            "limit": 1,
            "renderer": "chrome",
            "scrapeOptions": { "renderJs": false },
        }))
        .await;

    assert_eq!(
        res.status_code(),
        200,
        "renderJs:false must reach CrawlRequest and waive the pin check: {}",
        res.text()
    );
}

/// Control for the test above: the same pin WITHOUT `renderJs:false` still has
/// to be rejected, or the assertion above would pass for the trivial reason
/// that the pin check never runs.
#[tokio::test]
async fn v2_crawl_renderer_pin_without_render_js_still_400s() {
    let server = test_app();
    let res = server
        .post("/v2/crawl")
        .json(&json!({
            "url": "http://127.0.0.1:9/never-fetched",
            "limit": 1,
            "renderer": "chrome",
        }))
        .await;

    assert_eq!(res.status_code(), 400, "body: {}", res.text());
}

/// A non-boolean `scrapeOptions.renderJs` is rejected at the route, not
/// silently treated as absent.
#[tokio::test]
async fn v2_crawl_non_boolean_render_js_400s() {
    let server = test_app();
    let res = server
        .post("/v2/crawl")
        .json(&json!({
            "url": "http://127.0.0.1:9/never-fetched",
            "scrapeOptions": { "renderJs": "false" },
        }))
        .await;

    assert_eq!(res.status_code(), 400, "body: {}", res.text());
}
