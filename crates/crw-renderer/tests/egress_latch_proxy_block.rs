//! A latched host whose proxy returns a NON-429 error status must still fall
//! back to direct.
//!
//! The rescue arm originally fired only on a proxy transport failure, a hung
//! proxy, or a 429/`cf-mitigated` block. But the latch is a hint that can be
//! wrong (one transient 429 latches a host whose direct egress is fine), and a
//! proxy exit IP is often the WORSE egress: datacenter/residential ranges get a
//! 403 or 5xx wall from origins the box's own IP clears. If the proxy's 403 were
//! returned as-is, a falsely-latched host would fail while direct would have
//! served 200 — a scrape-success regression. So ANY non-2xx proxy response has
//! to leave the direct rescue reachable, not just the block signal.
//!
//! This lives in its own test binary so its proxy env does not race the
//! DEAD_PROXY value the sibling `egress_latch.rs` tests set in the same process.

use std::collections::HashMap;

use crw_core::Deadline;
use crw_core::config::{RendererConfig, RendererMode, StealthConfig};
use crw_renderer::FallbackRenderer;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn set_env(k: &str, v: &str) {
    // SAFETY: one binary per tests/*.rs, so this process owns its env.
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

/// Host is latched, so the proxy is tried first — and it answers 403 (an exit-IP
/// wall, not a 429). Direct must still be attempted and serve the page.
#[tokio::test]
async fn latched_host_falls_back_to_direct_when_the_proxy_walls_it_403() {
    set_env("CRW_ALLOW_LOOPBACK_FOR_TESTS", "1");

    // The proxy answers every request with 403 (it walls this host's exit IP).
    let proxy = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403).set_body_string("blocked at the proxy exit"))
        .mount(&proxy)
        .await;
    set_env("CRW_HTTP_RATELIMIT_PROXY_URL", &proxy.uri());

    // Direct egress serves the page fine.
    let origin = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>direct works</html>"))
        .mount(&origin)
        .await;
    let url = origin.uri();
    let host = url::Url::parse(&url)
        .unwrap()
        .host_str()
        .unwrap()
        .to_owned();

    crw_renderer::egress::global().note_block(&host).await;
    assert!(
        crw_renderer::egress::global().should_proxy(&host).await,
        "test precondition: host must be latched"
    );

    let before = crw_core::metrics::metrics().egress_latch_hit_total.get();

    let result = renderer()
        .fetch(
            &url,
            &HashMap::new(),
            Some(false),
            None,
            None,
            // 8s: above MIN_BUDGET_FOR_LATCH so the latch actually engages.
            Deadline::from_request_ms(8_000),
        )
        .await;

    assert!(
        result.is_ok(),
        "a latched host whose proxy answers 403 must still succeed over direct — \
         the latch reorders egress, it must never suppress it. Got: {:?}",
        result.err()
    );
    assert!(
        result.unwrap().html.contains("direct works"),
        "the rescued direct attempt's body must be what is returned, not the proxy's 403"
    );

    // The proxy really was tried first (latch engaged)...
    assert!(
        crw_core::metrics::metrics().egress_latch_hit_total.get() > before,
        "the latch must have engaged so the proxy was tried before direct"
    );
    assert!(
        !proxy.received_requests().await.unwrap().is_empty(),
        "the proxy-first attempt must actually hit the proxy"
    );
    // ...and direct really rescued it.
    assert!(
        !origin.received_requests().await.unwrap().is_empty(),
        "the direct rescue attempt must actually hit the origin"
    );
}
