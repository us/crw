//! A body-verdict IP-reputation block seen on DIRECT egress must latch the host.
//!
//! Wikimedia bans datacenter IPs with a `<body>`-less shell carrying a canonical
//! footer phrase and no distinguishing header, so neither the 429 arm nor the
//! challenge-header arm can see it. Without this latch every URL on such a host
//! re-climbs the whole doomed renderer ladder: measured 7-12s per page against
//! ~3s once the host is routed to residential egress first.
//!
//! The write is DIRECT-ONLY and that is load-bearing: a block observed through a
//! proxy says the *proxy* is blocked, and latching on it would re-arm the TTL on
//! every request, so the latch would never expire, direct would never be
//! re-probed (TTL expiry *is* the half-open probe) and the host would be pinned
//! to paid egress permanently.
//!
//! ONE test per binary, deliberately: the latch is keyed by
//! `preference::normalize_host`, which returns the bare IP for a literal, so
//! every wiremock origin in a process shares the single `127.0.0.1` key. Two
//! tests in one binary would race on it and pass or fail by scheduling order.

use std::collections::HashMap;

use crw_core::Deadline;
use crw_core::config::{RendererConfig, RendererMode, StealthConfig};
use crw_renderer::FallbackRenderer;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn renderer() -> FallbackRenderer {
    let cfg = RendererConfig {
        mode: RendererMode::None,
        ..Default::default()
    };
    FallbackRenderer::new(&cfg, "crw-test", None, &StealthConfig::default())
        .expect("renderer builds in http-only mode")
}

/// Fetch `body` from a throwaway origin over DIRECT egress and return its host.
async fn fetch_direct(body: &'static str, status: u16) -> String {
    // SAFETY: one binary per tests/*.rs, so this process owns its env. No proxy,
    // so the fetch is unambiguously direct and the latch verdict is meaningful.
    unsafe {
        std::env::set_var("CRW_ALLOW_LOOPBACK_FOR_TESTS", "1");
        std::env::remove_var("CRW_HTTP_RATELIMIT_PROXY_URL");
    }
    let origin = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(status).set_body_string(body))
        .mount(&origin)
        .await;
    let url = origin.uri();
    let _ = renderer()
        .fetch(
            &url,
            &HashMap::new(),
            Some(false),
            None,
            Some("auto"),
            Deadline::from_request_ms(15_000),
        )
        .await;
    url::Url::parse(&url)
        .unwrap()
        .host_str()
        .unwrap()
        .to_owned()
}

/// The real shape, reduced: no `<body>` tag, and the canonical Wikimedia footer
/// sentence that `looks_like_generic_bot_wall` matches on.
const WIKIMEDIA_BAN_SHELL: &str = concat!(
    "<!DOCTYPE html><html><head><title>Wikimedia Error</title></head>",
    "<div><h1>Our servers are currently under maintenance or experiencing a technical problem.</h1>",
    "<p>If you report this error to the Wikimedia System Administrators, please include ",
    "the details below.</p></div></html>"
);

#[tokio::test]
async fn ip_reputation_block_seen_on_direct_latches_the_host() {
    let host = fetch_direct(WIKIMEDIA_BAN_SHELL, 403).await;
    assert!(
        crw_renderer::egress::global().should_proxy(&host).await,
        "a datacenter-IP ban seen on direct egress must latch the host proxy-first"
    );
}
