//! Ignoring a site block in the breaker's failure window only pays off when some
//! tier downstream can actually clear it.
//!
//! In the common self-host build — no `chrome_proxy`, no stealth tier, no
//! `CRW_HTTP_RATELIMIT_PROXY_URL` — every tier egresses from the same banned IP,
//! so suppressing the breaker there would make a permanently blocked host re-walk
//! the whole serial ladder on every request (~3-6s/page against ~0.5s once the
//! breaker opens: a 6-12x slowdown on a crawl) while recovering exactly nothing.
//! So the suppression is gated on a recovery tier actually existing.
//!
//! Own test binary because it mutates process-global environment. `HttpFetcher`
//! reads `CRW_HTTP_RATELIMIT_PROXY_URL` and the `*_PROXY` variables when it is
//! CONSTRUCTED, and under the Rust 2024 contract `remove_var` racing a concurrent
//! `getenv` is undefined behaviour — which is exactly what would happen against
//! the ~19 sibling constructors in the crate's unit-test binary.

use crw_core::config::{CdpEndpoint, RendererConfig, RendererMode, StealthConfig};
use crw_renderer::FallbackRenderer;

/// Clear every egress-proxy signal so "no recovery tier" is really true, however
/// the developer's shell happens to be configured.
fn clear_proxy_env() {
    // SAFETY: one binary per tests/*.rs, so this process owns its env, and this
    // runs before any fetcher in it is constructed.
    unsafe {
        for k in [
            "CRW_HTTP_RATELIMIT_PROXY_URL",
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
        ] {
            std::env::remove_var(k);
        }
    }
}

fn chrome_only() -> RendererConfig {
    RendererConfig {
        mode: RendererMode::Auto,
        chrome: Some(CdpEndpoint {
            ws_url: "ws://127.0.0.1:9223/".into(),
        }),
        chrome_proxy: None,
        ..Default::default()
    }
}

/// All three cases in ONE test, run sequentially: they mutate the same process
/// environment, and `cargo test` would otherwise run them on parallel threads
/// where one's `clear_proxy_env` races another's construction.
#[test]
fn site_block_suppression_tracks_whether_a_recovery_tier_exists() {
    clear_proxy_env();
    let lean = FallbackRenderer::new(&chrome_only(), "crw-test", None, &StealthConfig::default())
        .expect("renderer builds");
    assert!(
        !lean.has_recovery_tier(),
        "no residential tier, no stealth tier and no fallback proxy: a site block \
         must keep counting, or a blocked host costs 6-12x for zero recall"
    );

    // Only meaningful with `cdp`: a lean build cannot CONSTRUCT chrome_proxy, so
    // `has_recovery_tier` is correctly false there however the config reads. That
    // is the point of deriving it from built state rather than from config.
    #[cfg(feature = "cdp")]
    {
        let with_residential = RendererConfig {
            chrome_proxy: Some(CdpEndpoint {
                ws_url: "ws://127.0.0.1:9224/".into(),
            }),
            ..chrome_only()
        };
        let r = FallbackRenderer::new(
            &with_residential,
            "crw-test",
            None,
            &StealthConfig::default(),
        )
        .expect("renderer builds");
        assert!(
            r.has_recovery_tier(),
            "chrome_proxy can clear an IP-reputation block, so blocks stop poisoning the breaker"
        );
    }

    // The fallback HTTP proxy is a recovery egress in its own right — it is what
    // turns the AWS-WAF 202 into a real page with no browser involved at all.
    // SAFETY: see `clear_proxy_env`.
    unsafe { std::env::set_var("CRW_HTTP_RATELIMIT_PROXY_URL", "http://127.0.0.1:18080") }
    let r = FallbackRenderer::new(&chrome_only(), "crw-test", None, &StealthConfig::default())
        .expect("renderer builds");
    assert!(
        r.has_recovery_tier(),
        "a usable fallback proxy is a recovery egress even with no residential CDP tier"
    );
}
