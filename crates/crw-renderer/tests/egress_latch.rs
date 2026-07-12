//! The egress latch must REORDER egress, never SUPPRESS direct.
//!
//! These tests guard the scrape-success red line. A latch is a hint learned from
//! one request ("this host hard-blocked our direct egress"), and it can be wrong:
//! a single transient 429 latches a host whose direct egress is actually fine.
//! If a latch could *forbid* direct, then any host whose proxy egress is worse —
//! proxy down, origin blocks the proxy's ranges, bad geo exit — would fail every
//! scrape for the whole cooldown. So a latched host must still be able to succeed
//! over direct when the proxy cannot deliver.

use std::collections::HashMap;

use crw_core::Deadline;
use crw_core::config::{RendererConfig, RendererMode, StealthConfig};
use crw_renderer::FallbackRenderer;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Port 1 is reserved and nothing listens there: every connection is refused
/// immediately, which is our stand-in for "the proxy is down/misconfigured".
const DEAD_PROXY: &str = "http://127.0.0.1:1";

fn set_env(k: &str, v: &str) {
    // SAFETY: these tests run in their own process (one binary per tests/*.rs).
    unsafe { std::env::set_var(k, v) }
}

fn renderer() -> FallbackRenderer {
    let cfg = RendererConfig {
        mode: RendererMode::None,
        ..Default::default()
    };
    FallbackRenderer::new(&cfg, "crw-test", None, &StealthConfig::default())
        .expect("renderer builds in http-only mode")
}

async fn serving_ok() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>direct works</html>"))
        .mount(&server)
        .await;
    server
}

/// THE red-line test. Host is latched, so we try the proxy first — but the proxy
/// is dead. Direct must still be attempted, and the fetch must SUCCEED.
///
/// Before the rescue arm existed, the dead proxy returned a transport error that
/// no arm handled, and the whole fetch failed. Every scrape of this host would
/// have failed for the entire 10-minute cooldown.
#[tokio::test]
async fn latched_host_falls_back_to_direct_when_the_proxy_is_dead() {
    set_env("CRW_ALLOW_LOOPBACK_FOR_TESTS", "1");
    set_env("CRW_HTTP_RATELIMIT_PROXY_URL", DEAD_PROXY);

    let server = serving_ok().await;
    let url = server.uri();
    let host = url::Url::parse(&url)
        .unwrap()
        .host_str()
        .unwrap()
        .to_owned();

    // Pretend a previous request learned this host blocks our direct egress.
    crw_renderer::egress::global().note_block(&host).await;
    assert!(
        crw_renderer::egress::global().should_proxy(&host).await,
        "test precondition: host must be latched"
    );

    let r = renderer();
    let result = r
        .fetch(
            &url,
            &HashMap::new(),
            Some(false),
            None,
            None,
            Deadline::from_request_ms(20_000),
        )
        .await;

    assert!(
        result.is_ok(),
        "a latched host with a dead proxy must still succeed over direct — the \
         latch reorders egress, it must never suppress it. Got: {:?}",
        result.err()
    );
    assert!(result.unwrap().html.contains("direct works"));

    // The origin really was reached directly.
    assert!(
        !server.received_requests().await.unwrap().is_empty(),
        "the direct rescue attempt must actually hit the origin"
    );
}

/// The latch must actually ENGAGE on the budget /map and /crawl really use.
///
/// This is the "is the feature dead code?" test. The threshold that protects the
/// 5s scrape path can very easily be set so high that it also excludes the 8s
/// per-page budget of /map — which is the one path the whole feature exists for.
/// A first attempt at 15s did exactly that.
///
/// Here the host is latched and the proxy is dead, on an 8s budget: the request
/// must have gone to the proxy FIRST (and then been rescued by direct). We prove
/// the proxy was really tried by asserting the latch-hit metric moved.
#[tokio::test]
async fn latch_engages_on_the_map_per_page_budget() {
    set_env("CRW_ALLOW_LOOPBACK_FOR_TESTS", "1");
    set_env("CRW_HTTP_RATELIMIT_PROXY_URL", DEAD_PROXY);

    let server = serving_ok().await;
    let url = server.uri();
    let host = url::Url::parse(&url)
        .unwrap()
        .host_str()
        .unwrap()
        .to_owned();
    crw_renderer::egress::global().note_block(&host).await;

    let before = crw_core::metrics::metrics().egress_latch_hit_total.get();

    let result = renderer()
        .fetch(
            &url,
            &HashMap::new(),
            Some(false),
            None,
            None,
            // config.effective_deadline_ms() default — what /map gives each page.
            // Comfortably above MIN_BUDGET_FOR_LATCH (6s), not sitting on it:
            // `remaining()` is always a hair under the budget it was built from, so
            // a threshold equal to this value would be decided by clock jitter.
            Deadline::from_request_ms(8_000),
        )
        .await;

    assert!(result.is_ok(), "direct rescue must still serve the page");

    let after = crw_core::metrics::metrics().egress_latch_hit_total.get();
    assert!(
        after > before,
        "the latch did NOT engage on an 8s budget — /map is the path this feature \
         exists for, so a threshold that excludes it makes the whole thing dead code"
    );
}
