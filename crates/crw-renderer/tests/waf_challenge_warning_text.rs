//! The customer-visible warning must name the vendor that actually blocked us.
//!
//! Own binary: this test requires NO proxy in the environment (see the sibling
//! `waf_challenge_*` tests, which set one).

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

/// The customer-visible warning must name the vendor that actually blocked us.
/// Before this change the string was hardcoded to Cloudflare and reached the API
/// caller through `crw_crawl::single`, so an AWS-WAF block on a CloudFront host
/// was reported as a Cloudflare one.
#[tokio::test]
async fn aws_waf_warning_does_not_claim_cloudflare() {
    set_env("CRW_ALLOW_LOOPBACK_FOR_TESTS", "1");
    // No proxy configured: the challenge survives to the returned result, which
    // is exactly the self-host shape we want to inspect.
    unsafe { std::env::remove_var("CRW_HTTP_RATELIMIT_PROXY_URL") }

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

    let warnings = result.warnings.join(" ");
    assert!(
        warnings.contains("AWS WAF"),
        "warning must name AWS WAF, got: {warnings:?}"
    );
    assert!(
        !warnings.to_lowercase().contains("cloudflare"),
        "an AWS WAF block must never be reported as Cloudflare, got: {warnings:?}"
    );
}
