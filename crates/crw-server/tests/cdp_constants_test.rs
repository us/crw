//! Regression test for issue #35: ensures the duplicated CDP overhead
//! constant in `crw-core::config` stays in lockstep with the source-of-truth
//! constants in `crw-renderer::cdp`. Without this, changing
//! `SPA_SELECTOR_MAX_MS`, the challenge retry budget, content-stability
//! budget, or fetch overhead in the renderer would silently re-introduce
//! deadline clamping that crushes per-tier timeouts.
//!
//! Why duplicate at all: `crw-renderer` already depends on `crw-core`, so
//! `crw-core` cannot import `crw_renderer::cdp` without a cycle.
//! The integration crate (`crw-server`) is the natural place to assert
//! equality because it depends on both.

#![cfg(feature = "cdp")]

#[test]
fn cdp_tier_overhead_matches_renderer_constants() {
    let renderer_sum_ms = crw_renderer::cdp::cdp_tier_overhead_ms();
    assert_eq!(
        renderer_sum_ms,
        crw_core::config::CDP_TIER_OVERHEAD_MS,
        "CDP_TIER_OVERHEAD_MS in crw-core::config drifted from crw-renderer::cdp constants. \
         Update crw_core::config::CDP_TIER_OVERHEAD_MS to match the new sum, or share via \
         a single source of truth."
    );
}
