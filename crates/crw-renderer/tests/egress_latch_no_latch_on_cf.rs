//! A fingerprint wall must NOT latch: a residential exit gets the same
//! Cloudflare managed challenge, so latching one would only burn paid bandwidth
//! for the whole cooldown while recovering nothing.
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

const CF_CHALLENGE: &str = concat!(
    "<html><body><h1>Just a moment...</h1>",
    "<script src=\"/cdn-cgi/challenge-platform/h/b/orchestrate/chl_page/v1\"></script>",
    "<div id=\"cf-please-wait\">Enable JavaScript and cookies to continue</div></body></html>"
);

#[tokio::test]
async fn cloudflare_challenge_does_not_latch() {
    let host = fetch_direct(CF_CHALLENGE, 403).await;
    assert!(
        !crw_renderer::egress::global().should_proxy(&host).await,
        "a Cloudflare managed challenge must not latch: residential egress cannot clear it"
    );
}
