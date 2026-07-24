//! AWS WAF Bot Control announces a challenge in a response header and serves it
//! as **HTTP 202 with a zero-length body**.
//!
//! That shape defeats every body-based detector — there is no markup to scan —
//! and 202 is not in the soft-block status list, so the empty response used to
//! be returned to the caller as a successful scrape with no content. Measured on
//! the production host: ballotpedia.org and jwa.org both answer this way and
//! both return a full page through residential egress.
//!
//! Its own test binary: `CRW_HTTP_RATELIMIT_PROXY_URL` is read when the fetcher
//! is CONSTRUCTED, so two tests setting different values in one process race.

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

const RECOVERED: &str =
    "<html><body><article>the real page, served through residential egress</article></body></html>";

/// The headline case: a 202 + `x-amzn-waf-action: challenge` + empty body must
/// arm the proxy retry in-request, and the caller gets the recovered page.
#[tokio::test]
async fn aws_waf_202_recovers_through_the_proxy() {
    set_env("CRW_ALLOW_LOOPBACK_FOR_TESTS", "1");

    let proxy = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(RECOVERED))
        .mount(&proxy)
        .await;
    set_env("CRW_HTTP_RATELIMIT_PROXY_URL", &proxy.uri());

    // The origin challenges: 202, no body, header names the action.
    let origin = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(202)
                .insert_header("x-amzn-waf-action", "challenge")
                .set_body_string(""),
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
        result.html.contains("the real page"),
        "the WAF challenge must be retried through the proxy, got: {:?}",
        result.html
    );
}
