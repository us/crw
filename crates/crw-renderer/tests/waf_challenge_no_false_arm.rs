//! Guard against the widened proxy arm over-firing: a healthy 202 that carries
//! real content must be returned as-is, never dragged through a proxy retry.
//!
//! Own binary: sets `CRW_HTTP_RATELIMIT_PROXY_URL`.

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

/// A bare 202 with real content is a perfectly good response and must not be
/// dragged through a proxy retry. Guards the widened arm against over-firing.
#[tokio::test]
async fn plain_202_with_content_is_left_alone() {
    set_env("CRW_ALLOW_LOOPBACK_FOR_TESTS", "1");

    // If the proxy is ever consulted it answers with a marker we can detect.
    let proxy = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html><body>PROXY</body></html>"))
        .mount(&proxy)
        .await;
    set_env("CRW_HTTP_RATELIMIT_PROXY_URL", &proxy.uri());

    let origin = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(202).set_body_string(
                "<html><body><article>accepted and rendered</article></body></html>",
            ),
        )
        .mount(&origin)
        .await;

    let result = renderer()
        .fetch(
            &origin.uri(),
            &HashMap::new(),
            Some(false),
            None,
            Some("auto"),
            Deadline::from_request_ms(15_000),
        )
        .await
        .expect("fetch succeeds");

    assert!(
        result.html.contains("accepted and rendered"),
        "a healthy 202 must be returned as-is"
    );
    assert!(
        !result.html.contains("PROXY"),
        "no proxy retry should have been armed"
    );
}
