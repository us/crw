//! Integration tests for `/map` discovery (`discover_urls`).
//!
//! These pin the behaviours that made `POST /v1/map` on news.ycombinator.com
//! burn 120s and return a 504 with zero URLs:
//!
//! - the BFS ran with no time budget whenever the sitemap was thin/absent, so
//!   the caller's timeout was the only bound — and it *dropped* the future,
//!   discarding every URL already found;
//! - robots.txt was fetched only for its `Sitemap:` lines, so links the site
//!   explicitly disallows (`/hide?`, `/vote?`) were fetched anyway, each through
//!   a full renderer ladder.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crw_core::config::{RendererConfig, RendererMode, StealthConfig};
use crw_crawl::crawl::{DiscoverOptions, discover_urls};
use crw_renderer::FallbackRenderer;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// wiremock binds to loopback, which the SSRF guard rejects by default.
fn allow_loopback() {
    // SAFETY: set before any discovery runs; tests in this file share one process.
    unsafe {
        std::env::set_var("CRW_ALLOW_LOOPBACK_FOR_TESTS", "1");
    }
}

/// An HTTP-only renderer: these tests exercise discovery, not JS rendering.
async fn renderer() -> Arc<FallbackRenderer> {
    allow_loopback();
    let cfg = RendererConfig {
        mode: RendererMode::None,
        ..Default::default()
    };
    Arc::new(
        FallbackRenderer::new(&cfg, "crw-test", None, &StealthConfig::default())
            .expect("renderer builds in http-only mode"),
    )
}

fn opts<'a>(
    base_url: &'a str,
    renderer: &'a Arc<FallbackRenderer>,
    overall_deadline: Instant,
    respect_robots: bool,
) -> DiscoverOptions<'a> {
    DiscoverOptions {
        base_url,
        max_depth: 2,
        use_sitemap: true,
        crawl_fallback: true,
        renderer,
        max_concurrency: 5,
        requests_per_second: 100.0,
        user_agent: "crw-test",
        proxy: None,
        deadline_ms_per_page: 5_000,
        per_host_max_concurrent: 4,
        url_filter: None,
        max_urls: 100,
        overall_deadline,
        respect_robots,
    }
}

/// robots.txt in the Hacker News shape: rules keyed on the QUERY string.
async fn mount_hn_like_site(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/robots.txt"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("User-Agent: *\nDisallow: /hide?\nDisallow: /vote?\n"),
        )
        .mount(server)
        .await;

    // No sitemap: this is what leaves the BFS as the only discovery path, and
    // what used to leave it completely unbudgeted.
    Mock::given(method("GET"))
        .and(path("/sitemap.xml"))
        .respond_with(ResponseTemplate::new(404))
        .mount(server)
        .await;

    let home = r#"
        <a href="/item?id=1">story</a>
        <a href="/hide?id=1&goto=news">hide</a>
        <a href="/vote?id=1&how=up">vote</a>
    "#;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(home))
        .mount(server)
        .await;

    for p in ["/item", "/hide", "/vote"] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>ok</html>"))
            .mount(server)
            .await;
    }
}

/// robots governs FETCHING, not LISTING: a disallowed link we discovered is
/// still reported to the caller, we simply never fetch it. Getting this wrong in
/// either direction is easy — dropping it from the output would silently shrink
/// every map result.
#[tokio::test]
async fn robots_disallowed_links_are_reported_but_never_fetched() {
    let server = MockServer::start().await;
    mount_hn_like_site(&server).await;
    let r = renderer().await;

    let result = discover_urls(opts(
        &server.uri(),
        &r,
        Instant::now() + Duration::from_secs(30),
        true,
    ))
    .await
    .expect("discovery succeeds");

    let has = |needle: &str| result.urls.iter().any(|u| u.contains(needle));
    assert!(
        has("/item"),
        "allowed link must be discovered: {:?}",
        result.urls
    );
    assert!(
        has("/hide"),
        "robots-disallowed link must still be REPORTED (robots forbids fetching, not listing): {:?}",
        result.urls
    );

    // The disallowed paths must never have been requested.
    let requests = server.received_requests().await.unwrap();
    let fetched_disallowed = requests
        .iter()
        .filter(|r| {
            let p = r.url.path();
            p.starts_with("/hide") || p.starts_with("/vote")
        })
        .count();
    assert_eq!(
        fetched_disallowed, 0,
        "robots.txt disallows /hide? and /vote? — they must never be fetched"
    );
}

/// With `respect_robots: false` the same links ARE fetched, proving the gate is
/// what stops them (and not some unrelated filter).
#[tokio::test]
async fn disallowed_links_are_fetched_when_robots_is_off() {
    let server = MockServer::start().await;
    mount_hn_like_site(&server).await;
    let r = renderer().await;

    discover_urls(opts(
        &server.uri(),
        &r,
        Instant::now() + Duration::from_secs(30),
        false,
    ))
    .await
    .expect("discovery succeeds");

    let requests = server.received_requests().await.unwrap();
    assert!(
        requests.iter().any(|r| r.url.path().starts_with("/hide")),
        "with robots off, the disallowed link should be fetched — otherwise this \
         test proves nothing about the gate"
    );
}

/// The regression test for the 504: a site with NO sitemap whose pages are slow.
/// The BFS used to get no budget at all here, so the caller's timeout fired,
/// dropped the future, and every URL found so far was thrown away.
///
/// Discovery must now stop at its own deadline and RETURN what it has.
#[tokio::test]
async fn slow_site_returns_partial_results_instead_of_timing_out() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/robots.txt"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/sitemap.xml"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"<a href="/a">a</a><a href="/b">b</a><a href="/c">c</a>"#),
        )
        .mount(&server)
        .await;
    // Every child page stalls far past the deadline.
    for p in ["/a", "/b", "/c"] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(30))
                    .set_body_string("<html>slow</html>"),
            )
            .mount(&server)
            .await;
    }

    let r = renderer().await;
    let started = Instant::now();
    let result = discover_urls(opts(
        &server.uri(),
        &r,
        Instant::now() + Duration::from_secs(3),
        true,
    ))
    .await
    .expect("discovery must SUCCEED with partial results, not fail with a timeout");

    assert!(
        started.elapsed() < Duration::from_secs(20),
        "discovery must stop at its own deadline, not run to the slow pages"
    );
    // The links were discovered from the home page even though fetching them stalled.
    assert!(
        result.urls.len() > 1,
        "partial results must still be returned: {:?}",
        result.urls
    );
}

/// A robots.txt that hangs must not be able to eat the whole budget and take
/// every result down with it: the robots fetch is clamped by the overall deadline.
#[tokio::test]
async fn hanging_robots_does_not_consume_the_whole_budget() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/robots.txt"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/sitemap.xml"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"<a href="/x">x</a>"#))
        .mount(&server)
        .await;

    let r = renderer().await;
    let started = Instant::now();
    let result = discover_urls(opts(
        &server.uri(),
        &r,
        Instant::now() + Duration::from_secs(3),
        true,
    ))
    .await
    .expect("a hanging robots.txt must not fail the whole call");

    assert!(
        started.elapsed() < Duration::from_secs(20),
        "robots fetch must be clamped by the overall deadline, took {:?}",
        started.elapsed()
    );
    assert!(!result.urls.is_empty(), "the base URL is always reported");
}

/// `max_urls` is a hard cap, not a suggestion. The base URL used to be appended
/// after the cap was enforced, so a caller asking for N could get N+1 back.
#[tokio::test]
async fn max_urls_is_never_exceeded() {
    let server = MockServer::start().await;
    mount_hn_like_site(&server).await;
    let r = renderer().await;

    let uri = server.uri();
    for limit in [1usize, 2, 3] {
        let mut o = opts(&uri, &r, Instant::now() + Duration::from_secs(30), true);
        o.max_urls = limit;
        let result = discover_urls(o).await.expect("discovery succeeds");
        assert!(
            result.urls.len() <= limit,
            "asked for {limit} URLs, got {}: {:?}",
            result.urls.len(),
            result.urls
        );
    }
}
