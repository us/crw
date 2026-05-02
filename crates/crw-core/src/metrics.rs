//! Prometheus metrics for renderer routing, host preferences, and circuit breakers.
//!
//! Counters and gauges are registered lazily on the default registry. Use
//! [`gather_text`] to render the current snapshot for `/metrics`.

use prometheus::{
    Encoder, IntCounterVec, IntGauge, Registry, TextEncoder,
    register_int_counter_vec_with_registry, register_int_gauge_with_registry,
};
use std::sync::OnceLock;

pub struct Metrics {
    pub registry: Registry,
    /// Outcome of routing decision per request: chosen renderer + decision kind.
    pub render_route_decision_total: IntCounterVec,
    /// Circuit breaker transitions to Open state, labeled by renderer.
    pub circuit_breaker_open_total: IntCounterVec,
    /// Host promoted to a heavier renderer by the auto-mode learner.
    pub host_preferences_promotions_total: IntCounterVec,
    /// Admin reset operations on the host preferences cache.
    pub admin_preferences_reset_total: IntCounterVec,
    /// User-pinned renderer requests (bypasses auto-mode learning).
    pub user_pin_total: IntCounterVec,
    /// Current size of the host preferences cache.
    pub host_preferences_size: IntGauge,
    /// Chrome navigation budget truncations, labeled by snapshot outcome
    /// (`ok` = partial DOM extracted, `empty` = nothing snapshotted).
    pub chrome_budget_truncated_total: IntCounterVec,
    /// Requests blocked by the chrome interception blocklist, labeled by
    /// reason (`resource_type`, `host`).
    pub chrome_blocked_requests_total: IntCounterVec,
    /// Outcomes ignored by the renderer circuit breaker (deadline-clamped,
    /// truncated-but-OK, etc.) — labeled by renderer and reason.
    pub breaker_ignored_total: IntCounterVec,
}

static METRICS: OnceLock<Metrics> = OnceLock::new();

pub fn metrics() -> &'static Metrics {
    METRICS.get_or_init(Metrics::new)
}

/// Eagerly register all metrics at boot. Forces `OnceLock` initialisation so
/// alert rules referencing series that have never emitted are evaluated
/// against present (zero-valued) series instead of absent ones.
pub fn init() {
    let _ = metrics();
}

impl Metrics {
    fn new() -> Self {
        let registry = Registry::new();
        let render_route_decision_total = register_int_counter_vec_with_registry!(
            "crw_render_route_decision_total",
            "Routing decisions by chosen renderer and decision kind",
            &["renderer", "decision"],
            registry
        )
        .unwrap();
        let circuit_breaker_open_total = register_int_counter_vec_with_registry!(
            "crw_circuit_breaker_open_total",
            "Circuit breaker transitions to Open, labeled by renderer and scope",
            &["renderer", "scope"],
            registry
        )
        .unwrap();
        let host_preferences_promotions_total = register_int_counter_vec_with_registry!(
            "crw_host_preferences_promotions_total",
            "Host preference promotions to a heavier renderer",
            &["from", "to"],
            registry
        )
        .unwrap();
        let admin_preferences_reset_total = register_int_counter_vec_with_registry!(
            "crw_admin_preferences_reset_total",
            "Admin resets of host preference state",
            &["scope"],
            registry
        )
        .unwrap();
        let user_pin_total = register_int_counter_vec_with_registry!(
            "crw_user_pin_total",
            "User-pinned renderer requests",
            &["renderer"],
            registry
        )
        .unwrap();
        let host_preferences_size = register_int_gauge_with_registry!(
            "crw_host_preferences_size",
            "Current size of the host preferences cache",
            registry
        )
        .unwrap();
        let chrome_budget_truncated_total = register_int_counter_vec_with_registry!(
            "crw_chrome_budget_truncated_total",
            "Chrome nav-budget truncations by snapshot outcome",
            &["outcome"],
            registry
        )
        .unwrap();
        let chrome_blocked_requests_total = register_int_counter_vec_with_registry!(
            "crw_chrome_blocked_requests_total",
            "Chrome requests blocked by interception, labeled by reason",
            &["reason"],
            registry
        )
        .unwrap();
        let breaker_ignored_total = register_int_counter_vec_with_registry!(
            "crw_breaker_ignored_total",
            "Renderer outcomes ignored by the circuit breaker (deadline-clamped, truncated, etc.)",
            &["renderer", "reason"],
            registry
        )
        .unwrap();
        Self {
            registry,
            render_route_decision_total,
            circuit_breaker_open_total,
            host_preferences_promotions_total,
            admin_preferences_reset_total,
            user_pin_total,
            host_preferences_size,
            chrome_budget_truncated_total,
            chrome_blocked_requests_total,
            breaker_ignored_total,
        }
    }
}

/// Render the current metrics snapshot in Prometheus text exposition format.
pub fn gather_text() -> String {
    let metric_families = metrics().registry.gather();
    let encoder = TextEncoder::new();
    let mut buf = Vec::new();
    encoder.encode(&metric_families, &mut buf).ok();
    String::from_utf8(buf).unwrap_or_default()
}
