//! The subtle rescue case: a latched host's proxy answers HTTP 200, but it is a
//! Cloudflare challenge (`cf-mitigated: challenge`).
//!
//! A pure `!is_success()` gate would let that interstitial through as the scrape
//! result, silently narrowing the very case the direct-side 429 arm already
//! treats as a block (`should_arm_proxy(200, true)`). The rescue must still fire
//! on the `cf-mitigated` signal so a latched host whose proxy exit hits a CF wall
//! the box's own IP clears is not stuck with the challenge page.
//!
//! Its own test binary: the proxy env must not race the other latch tests that
//! set a different `CRW_HTTP_RATELIMIT_PROXY_URL` in the same process.

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

#[tokio::test]
async fn latched_host_falls_back_to_direct_on_a_cf_challenge_served_as_200() {
    set_env("CRW_ALLOW_LOOPBACK_FOR_TESTS", "1");

    // The proxy answers 200, but the cf-mitigated header marks it a challenge.
    let proxy = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cf-mitigated", "challenge")
                .set_body_string("<html>just a moment... cloudflare challenge</html>"),
        )
        .mount(&proxy)
        .await;
    set_env("CRW_HTTP_RATELIMIT_PROXY_URL", &proxy.uri());

    // Direct egress serves the real page.
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

    let result = renderer()
        .fetch(
            &url,
            &HashMap::new(),
            Some(false),
            None,
            None,
            Deadline::from_request_ms(8_000),
        )
        .await;

    assert!(result.is_ok(), "the direct rescue must serve the page");
    assert!(
        result.unwrap().html.contains("direct works"),
        "a cf-mitigated 200 from the proxy must trigger the direct rescue, not be \
         returned as the challenge page"
    );
    assert!(
        !origin.received_requests().await.unwrap().is_empty(),
        "the direct rescue attempt must actually hit the origin"
    );
}
