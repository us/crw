//! The safety clause for the widened proxy arm: a challenge-armed proxy that
//! HANGS must not turn a soft empty response into a hard error.
//!
//! Before this, all three proxy-rescue arms were gated on `proxy_first`, which is
//! only true for a *latched* host. A proxy armed mid-loop by a 429 or a challenge
//! header therefore matched none of them: control fell through to the bare
//! `Err(_)` arm, which returns `CrwError::Timeout` after consuming the whole
//! deadline. Measured motivation: one ballotpedia fetch through the residential
//! exit took 38.2s. On a 15s search budget that arm would have converted today's
//! soft `202`-with-no-body into a 5xx — a scrape-success regression, on the very
//! requests this change exists to improve.
//!
//! Own test binary: sets `CRW_HTTP_RATELIMIT_PROXY_URL`, which is read when the
//! fetcher is CONSTRUCTED, so two tests setting different values in one process
//! would race.

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

#[tokio::test]
async fn hanging_challenge_proxy_falls_back_to_direct_instead_of_erroring() {
    // A proxy that never answers within any budget we would give it.
    let proxy = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_secs(60))
                .set_body_string("<html><body>too late to matter</body></html>"),
        )
        .mount(&proxy)
        .await;

    // SAFETY: one binary per tests/*.rs, so this process owns its env.
    unsafe {
        std::env::set_var("CRW_ALLOW_LOOPBACK_FOR_TESTS", "1");
        std::env::set_var("CRW_HTTP_RATELIMIT_PROXY_URL", proxy.uri());
    }

    // The origin answers instantly with the AWS-WAF challenge shape, which is
    // what arms the proxy in the first place.
    let origin = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(202)
                .insert_header("x-amzn-waf-action", "challenge")
                .set_body_string(""),
        )
        .mount(&origin)
        .await;

    let started = std::time::Instant::now();
    let result = renderer()
        .fetch(
            &origin.uri(),
            &HashMap::new(),
            Some(false),
            None,
            Some("auto"),
            // The real search-enrichment budget.
            Deadline::from_request_ms(15_000),
        )
        .await;

    let elapsed = started.elapsed();
    let fetched = result.expect(
        "a hanging challenge-armed proxy must fall back to direct, \
         not surface CrwError::Timeout",
    );
    assert_eq!(
        fetched.status_code, 202,
        "the direct rescue must return the origin's own response"
    );
    // A single challenge-armed proxy attempt is capped at CHALLENGE_PROXY_MAX
    // (6s), so a hanging exit costs that plus a fast direct rescue — NOT
    // `remaining - reserve` (13.5s of the 15s budget), which would have made a
    // bad exit almost as expensive as the whole request while still returning the
    // same empty body.
    assert!(
        elapsed < std::time::Duration::from_secs(9),
        "the hanging proxy attempt must be capped, not given the whole budget; took {elapsed:?}"
    );
}
