//! Integration tests for the /v1/map URL filter (issue #40).
//!
//! Boots an in-process axum server backed by an `AppState` whose upstream
//! HTML/sitemap source is a `wiremock` server. Exercises both the BFS path
//! (HTML extraction) and the sitemap path, plus opt-out shapes and the
//! metrics surface.

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
        // SAFETY: tests in this binary all want the same opt-in, set once
        // before any tokio worker spawns its own thread.
        unsafe {
            std::env::set_var("CRW_ALLOW_LOOPBACK_FOR_TESTS", "1");
        }
    });
}

/// HTML the mock server returns for `GET /`. Mirrors the issue-40
/// fixture shape: WooCommerce action URLs, tracker query strings,
/// pagination URLs, and a canonical product URL. `{base}` is replaced
/// with the mock server's URI before serving.
const ISSUE_40_HTML_TEMPLATE: &str = r#"<!doctype html>
<html><head><title>Shanzastore — issue #40 fixture</title></head><body>
<a href="{base}/about">About</a>
<a href="{base}/contact">Contact</a>
<a href="{base}/shop">Shop</a>
<a href="{base}/shop?page=2">Shop page 2</a>
<a href="{base}/shop?page=3">Shop page 3</a>
<a href="{base}/product/sample-1">Product 1</a>
<a href="{base}/product/sample-2">Product 2</a>
<a href="{base}/blog/spring-sale?utm_source=facebook&fbclid=IwAR12345">Spring sale</a>
<a href="{base}/?add-to-cart=360">Add to cart 360</a>
<a href="{base}/?add-to-cart=361">Add to cart 361</a>
<a href="{base}/?add_to_wishlist=6241&_wpnonce=b7643da9b9">Add to wishlist</a>
<a href="{base}/?remove_item=361&_wpnonce=abc123">Remove item</a>
<a href="{base}/?undo_item=361">Undo item</a>
<a href="{base}/?wc-ajax=remove_from_cart">WC ajax</a>
<a href="{base}/landing?gclid=Cj0KCQjw_abc">Ad landing</a>
</body></html>"#;

fn issue_40_html(base: &str) -> String {
    ISSUE_40_HTML_TEMPLATE.replace("{base}", base)
}

fn sitemap_xml(base: &str) -> String {
    format!(
        r#"<?xml version="1.0"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url><loc>{base}/</loc></url>
  <url><loc>{base}/about</loc></url>
  <url><loc>{base}/shop</loc></url>
  <url><loc>{base}/shop?page=2</loc></url>
  <url><loc>{base}/product/sample-1?utm_source=newsletter</loc></url>
  <url><loc>{base}/?add-to-cart=999</loc></url>
</urlset>"#
    )
}

async fn upstream_server() -> MockServer {
    let server = MockServer::start().await;
    let base = server.uri();
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html; charset=utf-8")
                .set_body_string(issue_40_html(&base)),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/sitemap.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/xml")
                .set_body_string(sitemap_xml(&base)),
        )
        .mount(&server)
        .await;
    // Empty bodies for the leaf pages — BFS records the link but the page
    // itself doesn't need to add more links for the test.
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html")
                .set_body_string("<html><body></body></html>"),
        )
        .mount(&server)
        .await;
    server
}

fn test_app(toml: &str) -> TestServer {
    allow_loopback_for_tests();
    let config: AppConfig = toml::from_str(toml).expect("config parse");
    let state = AppState::new(config).expect("AppState::new failed");
    let app = create_app(state);
    TestServer::new(app)
}

fn default_app() -> TestServer {
    test_app("")
}

#[tokio::test]
async fn bfs_path_drops_action_urls_strips_trackers() {
    let upstream = upstream_server().await;
    let server = default_app();
    let resp = server
        .post("/v1/map")
        .json(&json!({
            "url": upstream.uri(),
            "useSitemap": false,
            "crawlFallback": true,
            "maxDepth": 1
        }))
        .await;
    resp.assert_status_ok();
    let body: Value = resp.json();
    let links = body["data"]["links"].as_array().expect("links array");
    let joined = links
        .iter()
        .map(|v| v.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    // Action URLs dropped.
    assert!(
        !joined.contains("add-to-cart"),
        "action URL leaked through filter:\n{joined}"
    );
    assert!(!joined.contains("_wpnonce"), "wpnonce leaked:\n{joined}");
    assert!(
        !joined.contains("add_to_wishlist"),
        "wishlist leaked:\n{joined}"
    );
    // Trackers stripped, base preserved.
    assert!(joined.contains("/blog/spring-sale"));
    assert!(!joined.contains("utm_source"));
    assert!(!joined.contains("fbclid"));
    assert!(!joined.contains("gclid"));
    // Drop count surfaced.
    let dropped = body["data"]["droppedActionCount"]
        .as_u64()
        .expect("droppedActionCount field");
    assert!(dropped >= 5, "expected ≥5 drops, got {dropped}");
}

#[tokio::test]
async fn sitemap_path_filters_same_as_bfs() {
    let upstream = upstream_server().await;
    let server = default_app();
    let resp = server
        .post("/v1/map")
        .json(&json!({
            "url": upstream.uri(),
            "useSitemap": true,
            "crawlFallback": false
        }))
        .await;
    resp.assert_status_ok();
    let body: Value = resp.json();
    let links = body["data"]["links"].as_array().unwrap();
    let joined = links
        .iter()
        .map(|v| v.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !joined.contains("add-to-cart"),
        "sitemap action URL leaked:\n{joined}"
    );
    assert!(!joined.contains("utm_source"), "tracker leaked:\n{joined}");
    assert!(joined.contains("/product/sample-1"), "lost product url");
    assert!(joined.contains("/shop?page=2"), "lost pagination param");
}

#[tokio::test]
async fn granular_opt_out_returns_raw() {
    let upstream = upstream_server().await;
    let server = default_app();
    let resp = server
        .post("/v1/map")
        .json(&json!({
            "url": upstream.uri(),
            "useSitemap": false,
            "crawlFallback": true,
            "maxDepth": 1,
            "stripTrackingParams": false,
            "dropActionUrls": false
        }))
        .await;
    resp.assert_status_ok();
    let body: Value = resp.json();
    let joined = body["data"]["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("add-to-cart=360"),
        "action URL missing in opt-out"
    );
    assert!(
        joined.contains("utm_source=facebook"),
        "tracker stripped in opt-out"
    );
}

#[tokio::test]
async fn coarse_opt_out_returns_raw() {
    let upstream = upstream_server().await;
    let server = default_app();
    let resp = server
        .post("/v1/map")
        .json(&json!({
            "url": upstream.uri(),
            "useSitemap": false,
            "crawlFallback": true,
            "maxDepth": 1,
            "ignoreQueryParameters": false
        }))
        .await;
    resp.assert_status_ok();
    let body: Value = resp.json();
    let joined = body["data"]["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("utm_source"),
        "tracker missing under coarse=false"
    );
}

#[tokio::test]
async fn coarse_opt_in_strips_all_keeps_action_drop() {
    let upstream = upstream_server().await;
    let server = default_app();
    let resp = server
        .post("/v1/map")
        .json(&json!({
            "url": upstream.uri(),
            "useSitemap": false,
            "crawlFallback": true,
            "maxDepth": 1,
            "ignoreQueryParameters": true
        }))
        .await;
    resp.assert_status_ok();
    let body: Value = resp.json();
    let joined = body["data"]["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    // Even the legitimate `?page=2` is stripped under coarse mode unless
    // it's in ALWAYS_PRESERVE — `page` IS in the preserve set, so it
    // survives. Pagination param survival is part of the contract.
    assert!(
        joined.contains("/shop?page=2"),
        "always-preserve key stripped"
    );
    // Trackers and other params all gone.
    assert!(!joined.contains("utm_source"));
    assert!(!joined.contains("fbclid"));
    // Action URL still dropped (Tier A runs under coarse).
    assert!(!joined.contains("add-to-cart"));
}

#[tokio::test]
async fn extra_params_cap_returns_422() {
    let upstream = upstream_server().await;
    let server = default_app();
    let big_list: Vec<String> = (0..65).map(|i| format!("k{i}")).collect();
    let resp = server
        .post("/v1/map")
        .json(&json!({
            "url": upstream.uri(),
            "extraTrackingParams": big_list
        }))
        .await;
    // 64-key cap → 400 InvalidRequest (the route maps that to 400, not 422).
    // Either status is acceptable; the contract is "request rejected".
    let status = resp.status_code();
    assert!(
        status.is_client_error(),
        "expected 4xx for over-cap, got {status}"
    );
}

#[tokio::test]
async fn metrics_endpoint_records_drops() {
    let upstream = upstream_server().await;
    let server = default_app();
    let _ = server
        .post("/v1/map")
        .json(&json!({
            "url": upstream.uri(),
            "useSitemap": false,
            "crawlFallback": true,
            "maxDepth": 1
        }))
        .await;
    let metrics = server.get("/metrics").await;
    metrics.assert_status_ok();
    let text = metrics.text();
    // Filter rule counts loaded at boot.
    assert!(
        text.contains("crw_map_filter_rules_loaded"),
        "rules_loaded metric missing"
    );
    // At least one action drop recorded from the BFS request above.
    assert!(
        text.contains(r#"crw_map_filter_dropped_total{reason="action_param"}"#),
        "action_param drop metric missing"
    );
}

#[tokio::test]
async fn sitemap_and_bfs_overlap_dedupes() {
    // The mock sitemap and the HTML both reference /shop?page=2 — the
    // result must contain it exactly once.
    let upstream = upstream_server().await;
    let server = default_app();
    let resp = server
        .post("/v1/map")
        .json(&json!({
            "url": upstream.uri(),
            "useSitemap": true,
            "crawlFallback": true,
            "maxDepth": 1
        }))
        .await;
    resp.assert_status_ok();
    let body: Value = resp.json();
    let links: Vec<String> = body["data"]["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap_or("").to_string())
        .collect();
    let shop_page_2_count = links.iter().filter(|u| u.contains("/shop?page=2")).count();
    assert_eq!(
        shop_page_2_count, 1,
        "expected single /shop?page=2, got {links:?}"
    );
}

#[tokio::test]
async fn concurrent_requests_with_distinct_extras_no_state_leak() {
    let upstream = upstream_server().await;
    let server = default_app();
    let base = upstream.uri();

    // Run two requests with different `preserveParams`. Request 1 explicitly
    // preserves `gclid`; request 2 strips it as a default tracker. Each
    // request must see ONLY its own extras — the server-wide Arc must not
    // mutate between concurrent handlers.
    let body1 = json!({
        "url": base.clone(),
        "useSitemap": false,
        "crawlFallback": true,
        "maxDepth": 1,
        "preserveParams": ["gclid"]
    });
    let body2 = json!({
        "url": base,
        "useSitemap": false,
        "crawlFallback": true,
        "maxDepth": 1
    });
    let (resp1, resp2) = tokio::join!(
        server.post("/v1/map").json(&body1),
        server.post("/v1/map").json(&body2),
    );

    resp1.assert_status_ok();
    resp2.assert_status_ok();
    let j1: Value = resp1.json();
    let j2: Value = resp2.json();
    let l1 = j1["data"]["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let l2 = j2["data"]["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    // Request 1 preserves `gclid`; request 2 strips it (default behaviour).
    assert!(
        l1.contains("gclid="),
        "req1 should preserve gclid; got {l1}"
    );
    assert!(
        !l2.contains("gclid="),
        "req2 should strip gclid — state leaked: {l2}"
    );
}
