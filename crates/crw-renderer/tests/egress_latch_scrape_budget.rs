//! The latch must stay INERT on the SaaS scrape budget.
//!
//! Own test binary on purpose: this asserts a process-global metric counter did
//! NOT move, so a sibling test that legitimately increments it would race this one
//! and make it flaky. Cargo gives each `tests/*.rs` its own process.

use std::collections::HashMap;

use crw_core::Deadline;
use crw_core::config::{RendererConfig, RendererMode, StealthConfig};
use crw_renderer::FallbackRenderer;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A proxy that accepts the TCP connection and then never answers — the pathology
/// a plain "connection refused" cannot exercise, and the one that could starve the
/// direct rescue if the latch engaged on a short budget.
fn blackholing_proxy() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        while let Ok((sock, _)) = listener.accept() {
            held.push(sock); // never read, never respond
        }
    });
    format!("http://{addr}")
}

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

/// On the 5s scrape budget the latch must NOT engage, even for a latched host with
/// a BLACKHOLING proxy in front of it.
///
/// If it did engage, that hanging proxy would eat part of the 5s and a healthy but
/// not-instant direct origin could no longer fit in the remainder — turning a
/// request that used to succeed into a failure, and pushing scrape success below
/// the 89.7% red line.
///
/// The assertion is on the METRIC, not on wall-clock timing: a "direct still
/// finished in time" assertion has to sit near the budget boundary to be meaningful,
/// which makes it a coin-flip under parallel test load. The counter says plainly
/// whether the latch fired.
#[tokio::test]
async fn latch_does_not_engage_on_the_scrape_budget() {
    set_env("CRW_ALLOW_LOOPBACK_FOR_TESTS", "1");
    set_env("CRW_HTTP_RATELIMIT_PROXY_URL", &blackholing_proxy());

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
            // The SaaS scrape deadline.
            Deadline::from_request_ms(5_000),
        )
        .await;

    assert!(
        result.is_ok(),
        "the scrape path must keep working: {:?}",
        result.err()
    );
    assert_eq!(
        crw_core::metrics::metrics().egress_latch_hit_total.get(),
        before,
        "the latch must stay inert on the 5s scrape budget — engaging there is how a \
         hanging proxy could starve direct and push scrape success below 89.7%"
    );
}
