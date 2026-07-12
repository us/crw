//! The egress latch must be INERT when no proxy is configured.
//!
//! Own test binary on purpose: the proxy client is built from a process-global
//! env var at renderer-construction time, so a sibling test that *sets* that var
//! would race this one that needs it *unset*. Cargo gives each `tests/*.rs` its
//! own process, which is the only way to keep both honest.

use std::collections::HashMap;

use crw_core::Deadline;
use crw_core::config::{RendererConfig, RendererMode, StealthConfig};
use crw_renderer::FallbackRenderer;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn set_env(k: &str, v: &str) {
    // SAFETY: this test owns its process.
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

/// With no proxy configured the latch is inert: it must not change anything on
/// its own. An unlatched-vs-latched host should behave identically here.
#[tokio::test]
async fn latch_is_inert_when_no_proxy_is_configured() {
    set_env("CRW_ALLOW_LOOPBACK_FOR_TESTS", "1");
    // No CRW_HTTP_RATELIMIT_PROXY_URL is ever set in this process.

    let server = serving_ok().await;
    let url = server.uri();
    let host = url::Url::parse(&url)
        .unwrap()
        .host_str()
        .unwrap()
        .to_owned();

    crw_renderer::egress::global().note_block(&host).await;

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
        "with no proxy configured a latched host must behave exactly as before: {:?}",
        result.err()
    );
    assert!(result.unwrap().html.contains("direct works"));
}
