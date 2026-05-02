//! Sliding-window circuit breaker with multi-probe half-open and
//! linear-back-off cooldown. See `plans/breaker-cascade-fix.md` Iter 3.
//!
//! ## States
//!
//! - `Closed`: allow all. Outcomes update a fixed-size sliding window;
//!   trip when `failure_rate >= threshold` and `call_count >= min_calls`.
//! - `Open { until, ejection_count }`: reject all until cooldown expires.
//!   Cooldown grows linearly with `ejection_count` capped at `max_cooldown`.
//! - `HalfOpen { admitted, succeeded, failed, opened_at }`: admit up to
//!   `max_probes` callers. Decision when `succeeded + failed == max_probes`
//!   OR `opened_at.elapsed() > eval_timeout`. Close iff
//!   `succeeded / max_probes >= half_open_success_rate`.
//!
//! ## Outcome classification
//!
//! Callers do not pass a raw `success: bool`. They report a
//! [`BreakerOutcome`] which distinguishes deadline-clamped attempts (parent
//! end-to-end deadline ate the budget) from genuine tier failures. Only
//! `TierTimeout`/`ConnectionError`/`RenderError` advance the failure window;
//! `DeadlineClamped` is observed via `crw_breaker_ignored_total` only.
//! `Truncated` is configurable (default ignored — chrome partial-DOM is a
//! feature, not a tier failure).

use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct BreakerConfig {
    pub window_size: usize,
    pub min_calls: usize,
    pub failure_rate_threshold: f64,
    pub base_cooldown: Duration,
    pub max_cooldown: Duration,
    pub max_probes: u32,
    pub half_open_success_rate: f64,
    pub eval_timeout: Duration,
    pub ejection_reset_after_closed: Duration,
    pub count_truncated_as_failure: bool,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        Self {
            window_size: 50,
            min_calls: 20,
            failure_rate_threshold: 0.55,
            base_cooldown: Duration::from_secs(10),
            max_cooldown: Duration::from_secs(120),
            max_probes: 5,
            half_open_success_rate: 0.60,
            eval_timeout: Duration::from_secs(30),
            ejection_reset_after_closed: Duration::from_secs(120),
            count_truncated_as_failure: false,
        }
    }
}

/// Internal classifier output. Callers compute this from `Result + AttemptContext`
/// at the recording boundary; the breaker treats only the explicit failure
/// classes as window-advancing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerOutcome {
    Success,
    Truncated,
    /// End-to-end deadline ate the budget before this tier could finish.
    /// Not the tier's fault; do not advance the window.
    DeadlineClamped,
    /// Tier-local timeout with full budget available — counts as failure.
    TierTimeout,
    ConnectionError,
    RenderError,
}

impl BreakerOutcome {
    fn is_failure(&self, count_truncated_as_failure: bool) -> bool {
        match self {
            BreakerOutcome::Success => false,
            BreakerOutcome::Truncated => count_truncated_as_failure,
            BreakerOutcome::DeadlineClamped => false,
            BreakerOutcome::TierTimeout
            | BreakerOutcome::ConnectionError
            | BreakerOutcome::RenderError => true,
        }
    }

    /// True if this outcome should advance the failure window at all.
    /// `DeadlineClamped` is fully ignored (only counted in observability).
    /// `Truncated` is conditionally ignored.
    fn advances_window(&self, count_truncated_as_failure: bool) -> bool {
        match self {
            BreakerOutcome::DeadlineClamped => false,
            BreakerOutcome::Truncated => count_truncated_as_failure,
            _ => true,
        }
    }

    pub fn ignored_reason(&self) -> Option<&'static str> {
        match self {
            BreakerOutcome::DeadlineClamped => Some("deadline_clamped"),
            BreakerOutcome::Truncated => Some("truncated"),
            _ => None,
        }
    }
}

/// Captured pre-call so the post-await classification is immune to
/// clock drift in the deadline branch (Codex C3 race fix).
#[derive(Debug, Clone, Copy)]
pub struct AttemptContext {
    pub remaining_at_start: Duration,
    pub tier_budget: Duration,
    pub was_clamped_by_deadline: bool,
}

impl AttemptContext {
    pub fn capture(remaining: Duration, tier_budget: Duration) -> Self {
        Self {
            remaining_at_start: remaining,
            tier_budget,
            was_clamped_by_deadline: tier_budget > remaining,
        }
    }
}

/// Classify a tier-attempt result into a BreakerOutcome. Callers must
/// supply the AttemptContext captured *before* the call so deadline
/// classification is deterministic regardless of post-await wall time.
pub fn classify_outcome(
    success: bool,
    is_truncated: bool,
    error_was_timeout: bool,
    ctx: &AttemptContext,
) -> BreakerOutcome {
    if success {
        if is_truncated {
            BreakerOutcome::Truncated
        } else {
            BreakerOutcome::Success
        }
    } else if error_was_timeout {
        if ctx.was_clamped_by_deadline {
            BreakerOutcome::DeadlineClamped
        } else {
            BreakerOutcome::TierTimeout
        }
    } else {
        BreakerOutcome::RenderError
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowSlot {
    Empty,
    Success,
    Failure,
}

#[derive(Debug)]
struct Window {
    ring: Vec<WindowSlot>,
    cursor: usize,
}

impl Window {
    fn new(size: usize) -> Self {
        Self {
            ring: vec![WindowSlot::Empty; size.max(1)],
            cursor: 0,
        }
    }
    fn push(&mut self, slot: WindowSlot) {
        let size = self.ring.len();
        self.ring[self.cursor] = slot;
        self.cursor = (self.cursor + 1) % size;
    }
    fn call_count(&self) -> usize {
        self.ring
            .iter()
            .filter(|s| **s != WindowSlot::Empty)
            .count()
    }
    fn failure_count(&self) -> usize {
        self.ring
            .iter()
            .filter(|s| **s == WindowSlot::Failure)
            .count()
    }
    fn failure_rate(&self) -> f64 {
        let calls = self.call_count();
        if calls == 0 {
            0.0
        } else {
            self.failure_count() as f64 / calls as f64
        }
    }
    fn clear(&mut self) {
        for s in self.ring.iter_mut() {
            *s = WindowSlot::Empty;
        }
        self.cursor = 0;
    }
}

#[derive(Debug)]
enum State {
    Closed {
        closed_since: Instant,
    },
    Open {
        until: Instant,
    },
    HalfOpen {
        admitted: u32,
        succeeded: u32,
        failed: u32,
        opened_at: Instant,
    },
}

#[derive(Debug)]
struct Inner {
    state: State,
    window: Window,
    ejection_count: u32,
}

/// Outcome of `try_acquire` — caller must respect this before calling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permit {
    /// Closed or recovering — caller may proceed.
    Allowed,
    /// HalfOpen probe granted — caller is one of up to `max_probes`.
    Probe,
    /// Open / probe quota exhausted — caller must skip this renderer.
    Rejected,
}

#[derive(Debug)]
pub struct CircuitBreaker {
    config: BreakerConfig,
    inner: Mutex<Inner>,
}

impl CircuitBreaker {
    pub fn new(config: BreakerConfig) -> Self {
        Self {
            inner: Mutex::new(Inner {
                state: State::Closed {
                    closed_since: Instant::now(),
                },
                window: Window::new(config.window_size),
                ejection_count: 0,
            }),
            config,
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(BreakerConfig::default())
    }

    fn current_cooldown(&self, ejection_count: u32) -> Duration {
        let mult = ejection_count.max(1);
        let dur = self.config.base_cooldown.saturating_mul(mult);
        std::cmp::min(dur, self.config.max_cooldown)
    }

    /// Lazy state evaluation: handles
    /// - Open → HalfOpen transition when cooldown elapses
    /// - HalfOpen eval timeout (force decision on partial probes)
    /// - ejection_count reset after sustained Closed period
    fn lazy_evaluate(&self, inner: &mut Inner) {
        match inner.state {
            State::Open { until } if Instant::now() >= until => {
                inner.state = State::HalfOpen {
                    admitted: 0,
                    succeeded: 0,
                    failed: 0,
                    opened_at: Instant::now(),
                };
            }
            State::HalfOpen {
                admitted,
                succeeded,
                failed,
                opened_at,
            } if opened_at.elapsed() > self.config.eval_timeout
                && (succeeded + failed) < admitted.max(self.config.max_probes) =>
            {
                // Partial probes — force decision.
                if succeeded == 0 {
                    // No evidence of recovery → reopen with grown ejection_count.
                    inner.ejection_count = inner.ejection_count.saturating_add(1);
                    let cooldown = self.current_cooldown(inner.ejection_count);
                    inner.state = State::Open {
                        until: Instant::now() + cooldown,
                    };
                } else {
                    // At least one success — close (partial evidence).
                    inner.state = State::Closed {
                        closed_since: Instant::now(),
                    };
                    inner.window.clear();
                }
            }
            State::Closed { closed_since }
                if inner.ejection_count > 0
                    && closed_since.elapsed() >= self.config.ejection_reset_after_closed =>
            {
                inner.ejection_count = 0;
            }
            _ => {}
        }
    }

    pub fn try_acquire(&self) -> Permit {
        let mut inner = self.inner.lock().expect("breaker mutex poisoned");
        self.lazy_evaluate(&mut inner);
        match inner.state {
            State::Closed { .. } => Permit::Allowed,
            State::Open { .. } => Permit::Rejected,
            State::HalfOpen { admitted, .. } if admitted < self.config.max_probes => {
                if let State::HalfOpen {
                    ref mut admitted, ..
                } = inner.state
                {
                    *admitted += 1;
                }
                Permit::Probe
            }
            State::HalfOpen { .. } => Permit::Rejected,
        }
    }

    /// Record an outcome. Returns `true` if this call transitioned the
    /// breaker into Open (caller may emit a metric).
    pub fn record_outcome(&self, outcome: BreakerOutcome) -> bool {
        let mut inner = self.inner.lock().expect("breaker mutex poisoned");
        self.lazy_evaluate(&mut inner);

        let advances = outcome.advances_window(self.config.count_truncated_as_failure);
        let is_failure = outcome.is_failure(self.config.count_truncated_as_failure);

        match inner.state {
            State::HalfOpen {
                ref mut admitted,
                ref mut succeeded,
                ref mut failed,
                ..
            } => {
                if !advances {
                    // Ignored outcome during half-open: free the slot we admitted
                    // but don't count it toward the decision.
                    *admitted = admitted.saturating_sub(1);
                    return false;
                }
                if is_failure {
                    *failed += 1;
                } else {
                    *succeeded += 1;
                }
                let total = *succeeded + *failed;
                let cap = self.config.max_probes;
                if total >= cap {
                    let success_rate = *succeeded as f64 / cap as f64;
                    if success_rate >= self.config.half_open_success_rate {
                        inner.state = State::Closed {
                            closed_since: Instant::now(),
                        };
                        inner.window.clear();
                        false
                    } else {
                        inner.ejection_count = inner.ejection_count.saturating_add(1);
                        let cooldown = self.current_cooldown(inner.ejection_count);
                        inner.state = State::Open {
                            until: Instant::now() + cooldown,
                        };
                        true
                    }
                } else {
                    false
                }
            }
            State::Closed { .. } => {
                if !advances {
                    return false;
                }
                if is_failure {
                    inner.window.push(WindowSlot::Failure);
                } else {
                    inner.window.push(WindowSlot::Success);
                }
                if inner.window.call_count() >= self.config.min_calls
                    && inner.window.failure_rate() >= self.config.failure_rate_threshold
                {
                    inner.ejection_count = inner.ejection_count.saturating_add(1);
                    let cooldown = self.current_cooldown(inner.ejection_count);
                    inner.state = State::Open {
                        until: Instant::now() + cooldown,
                    };
                    inner.window.clear();
                    true
                } else {
                    false
                }
            }
            State::Open { .. } => false,
        }
    }

    /// Release a probe permit without recording an outcome. Decrements
    /// the half-open admitted counter so the slot frees for retry.
    pub fn cancel_probe(&self) {
        let mut inner = self.inner.lock().expect("breaker mutex poisoned");
        if let State::HalfOpen {
            ref mut admitted, ..
        } = inner.state
        {
            *admitted = admitted.saturating_sub(1);
        }
    }

    pub fn is_open(&self) -> bool {
        let mut inner = self.inner.lock().expect("breaker mutex poisoned");
        self.lazy_evaluate(&mut inner);
        matches!(inner.state, State::Open { .. })
    }

    /// Snapshot for the debug endpoint: state label + cooldown remaining
    /// (Some only when Open) + ejection count + current window stats.
    pub fn snapshot(&self) -> BreakerSnapshot {
        let mut inner = self.inner.lock().expect("breaker mutex poisoned");
        self.lazy_evaluate(&mut inner);
        let (label, opens_in) = match inner.state {
            State::Closed { .. } => ("closed", None),
            State::HalfOpen { .. } => ("half_open", None),
            State::Open { until } => {
                let remaining = until.saturating_duration_since(Instant::now()).as_secs();
                ("open", Some(remaining))
            }
        };
        BreakerSnapshot {
            state: label,
            opens_in_seconds: opens_in,
            ejection_count: inner.ejection_count,
            window_call_count: inner.window.call_count() as u32,
            window_failure_rate: inner.window.failure_rate(),
        }
    }

    /// Reset all state to Closed with empty window. Used by
    /// `POST /admin/breakers/reset`.
    pub fn reset(&self) {
        let mut inner = self.inner.lock().expect("breaker mutex poisoned");
        inner.state = State::Closed {
            closed_since: Instant::now(),
        };
        inner.window.clear();
        inner.ejection_count = 0;
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BreakerSnapshot {
    pub state: &'static str,
    pub opens_in_seconds: Option<u64>,
    pub ejection_count: u32,
    pub window_call_count: u32,
    pub window_failure_rate: f64,
}

// ── Registry: per-host + global per-renderer breakers ────────────────

use crate::preference::normalize_host;
use crw_core::metrics::metrics;
use crw_core::types::RendererKind;
use moka::future::Cache;
use std::sync::Arc;

const REGISTRY_CAPACITY: u64 = 10_000;
const REGISTRY_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone)]
pub struct BreakerRegistry {
    config: BreakerConfig,
    global: Arc<[(RendererKind, Arc<CircuitBreaker>); 3]>,
    host: Cache<(String, RendererKind), Arc<CircuitBreaker>>,
}

impl BreakerRegistry {
    pub fn new(config: BreakerConfig) -> Self {
        let global = Arc::new([
            (RendererKind::Http, Arc::new(CircuitBreaker::new(config))),
            (
                RendererKind::Lightpanda,
                Arc::new(CircuitBreaker::new(config)),
            ),
            (RendererKind::Chrome, Arc::new(CircuitBreaker::new(config))),
        ]);
        let host = Cache::builder()
            .max_capacity(REGISTRY_CAPACITY)
            .time_to_idle(REGISTRY_TTL)
            .build();
        Self {
            config,
            global,
            host,
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(BreakerConfig::default())
    }

    pub fn config(&self) -> BreakerConfig {
        self.config
    }

    pub fn global_for(&self, renderer: RendererKind) -> Arc<CircuitBreaker> {
        for (kind, breaker) in self.global.iter() {
            if *kind == renderer {
                return Arc::clone(breaker);
            }
        }
        unreachable!("RendererKind is closed: Http | Lightpanda | Chrome")
    }

    pub async fn host_for(&self, host: &str, renderer: RendererKind) -> Arc<CircuitBreaker> {
        let key = (normalize_host(host), renderer);
        let cfg = self.config;
        self.host
            .get_with(key, async move { Arc::new(CircuitBreaker::new(cfg)) })
            .await
    }

    pub async fn try_acquire(&self, host: &str, renderer: RendererKind) -> Permit {
        let global = self.global_for(renderer);
        let host_b = self.host_for(host, renderer).await;
        let g = global.try_acquire();
        if g == Permit::Rejected {
            return Permit::Rejected;
        }
        let h = host_b.try_acquire();
        if h == Permit::Rejected {
            if g == Permit::Probe {
                global.cancel_probe();
            }
            return Permit::Rejected;
        }
        if g == Permit::Probe || h == Permit::Probe {
            Permit::Probe
        } else {
            Permit::Allowed
        }
    }

    /// Record outcome to both tiers. Increments
    /// `circuit_breaker_open_total` on transitions to Open and emits
    /// `crw_breaker_ignored_total{reason}` for non-window-advancing outcomes.
    pub async fn record_outcome(
        &self,
        host: &str,
        renderer: RendererKind,
        outcome: BreakerOutcome,
    ) {
        if let Some(reason) = outcome.ignored_reason() {
            metrics()
                .breaker_ignored_total
                .with_label_values(&[renderer.as_str(), reason])
                .inc();
        }
        let g_tripped = self.global_for(renderer).record_outcome(outcome);
        let h_tripped = self.host_for(host, renderer).await.record_outcome(outcome);
        if g_tripped {
            metrics()
                .circuit_breaker_open_total
                .with_label_values(&[renderer.as_str(), "global"])
                .inc();
        }
        if h_tripped {
            metrics()
                .circuit_breaker_open_total
                .with_label_values(&[renderer.as_str(), "host"])
                .inc();
        }
    }

    /// Convenience for legacy bool-call sites that don't yet have full
    /// outcome classification. Maps `true → Success`, `false → RenderError`.
    pub async fn record_result(&self, host: &str, renderer: RendererKind, success: bool) {
        let outcome = if success {
            BreakerOutcome::Success
        } else {
            BreakerOutcome::RenderError
        };
        self.record_outcome(host, renderer, outcome).await;
    }

    pub async fn cancel_probe(&self, host: &str, renderer: RendererKind) {
        self.global_for(renderer).cancel_probe();
        self.host_for(host, renderer).await.cancel_probe();
    }

    /// Reset every breaker to Closed and clear the host cache. Used by
    /// `POST /admin/breakers/reset`. Returns the count of host entries
    /// that were evicted so callers can log the audit signal.
    pub fn reset_all(&self) -> u64 {
        for (_, breaker) in self.global.iter() {
            breaker.reset();
        }
        let count = self.host.entry_count();
        self.host.invalidate_all();
        count
    }

    pub fn snapshot(&self) -> RegistrySnapshot {
        let global: Vec<BreakerStatus> = self
            .global
            .iter()
            .map(|(kind, breaker)| {
                let snap = breaker.snapshot();
                BreakerStatus {
                    renderer: kind.as_str().to_string(),
                    state: snap.state.to_string(),
                    opens_in_seconds: snap.opens_in_seconds,
                    ejection_count: snap.ejection_count,
                    window_call_count: snap.window_call_count,
                    window_failure_rate: snap.window_failure_rate,
                }
            })
            .collect();
        let mut per_host: Vec<HostBreakerStatus> = Vec::new();
        for (key, breaker) in self.host.iter() {
            let snap = breaker.snapshot();
            per_host.push(HostBreakerStatus {
                host: key.0.clone(),
                renderer: key.1.as_str().to_string(),
                state: snap.state.to_string(),
                opens_in_seconds: snap.opens_in_seconds,
                ejection_count: snap.ejection_count,
                window_call_count: snap.window_call_count,
                window_failure_rate: snap.window_failure_rate,
            });
        }
        per_host.sort_by(|a, b| {
            a.host
                .cmp(&b.host)
                .then_with(|| a.renderer.cmp(&b.renderer))
        });
        RegistrySnapshot { global, per_host }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BreakerStatus {
    pub renderer: String,
    pub state: String,
    pub opens_in_seconds: Option<u64>,
    pub ejection_count: u32,
    pub window_call_count: u32,
    pub window_failure_rate: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct HostBreakerStatus {
    pub host: String,
    pub renderer: String,
    pub state: String,
    pub opens_in_seconds: Option<u64>,
    pub ejection_count: u32,
    pub window_call_count: u32,
    pub window_failure_rate: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RegistrySnapshot {
    pub global: Vec<BreakerStatus>,
    pub per_host: Vec<HostBreakerStatus>,
}

impl Default for BreakerRegistry {
    fn default() -> Self {
        Self::with_defaults()
    }
}

/// RAII guard for a HalfOpen probe permit. Drop without `disarm` →
/// `cancel_probe` decrements the half-open admitted slot.
pub struct ProbeGuard {
    global: Option<Arc<CircuitBreaker>>,
    host: Option<Arc<CircuitBreaker>>,
    armed: bool,
}

impl ProbeGuard {
    pub fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for ProbeGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Some(g) = &self.global {
            g.cancel_probe();
        }
        if let Some(h) = &self.host {
            h.cancel_probe();
        }
    }
}

impl BreakerRegistry {
    pub async fn acquire_with_guard(
        &self,
        host: &str,
        renderer: RendererKind,
    ) -> (Permit, ProbeGuard) {
        let permit = self.try_acquire(host, renderer).await;
        let (global, host_b) = if matches!(permit, Permit::Probe) {
            (
                Some(self.global_for(renderer)),
                Some(self.host_for(host, renderer).await),
            )
        } else {
            (None, None)
        };
        let guard = ProbeGuard {
            global,
            host: host_b,
            armed: matches!(permit, Permit::Probe),
        };
        (permit, guard)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small_cfg() -> BreakerConfig {
        BreakerConfig {
            window_size: 10,
            min_calls: 5,
            failure_rate_threshold: 0.5,
            base_cooldown: Duration::from_millis(20),
            max_cooldown: Duration::from_millis(200),
            max_probes: 3,
            half_open_success_rate: 0.6,
            eval_timeout: Duration::from_millis(500),
            ejection_reset_after_closed: Duration::from_millis(100),
            count_truncated_as_failure: false,
        }
    }

    fn fail() -> BreakerOutcome {
        BreakerOutcome::RenderError
    }
    fn ok() -> BreakerOutcome {
        BreakerOutcome::Success
    }

    #[test]
    fn closed_allows_all() {
        let b = CircuitBreaker::new(small_cfg());
        for _ in 0..20 {
            assert_eq!(b.try_acquire(), Permit::Allowed);
            b.record_outcome(ok());
        }
    }

    #[test]
    fn does_not_trip_below_min_calls() {
        let b = CircuitBreaker::new(small_cfg());
        // 4 failures = below min_calls (5) → no trip.
        for _ in 0..4 {
            assert_eq!(b.try_acquire(), Permit::Allowed);
            b.record_outcome(fail());
        }
        assert!(!b.is_open());
    }

    #[test]
    fn trips_when_window_majority_fails() {
        let b = CircuitBreaker::new(small_cfg());
        // 5 fail + 0 success = 100% failure ≥ 50%, ≥ min_calls=5 → trip.
        for _ in 0..5 {
            b.try_acquire();
            b.record_outcome(fail());
        }
        assert!(b.is_open());
    }

    #[test]
    fn single_failure_does_not_trip() {
        let b = CircuitBreaker::new(small_cfg());
        for _ in 0..4 {
            b.try_acquire();
            b.record_outcome(ok());
        }
        b.try_acquire();
        b.record_outcome(fail());
        // 1/5 = 20% < 50% → no trip.
        assert!(!b.is_open());
    }

    #[test]
    fn deadline_clamped_does_not_trip() {
        let b = CircuitBreaker::new(small_cfg());
        for _ in 0..30 {
            b.try_acquire();
            b.record_outcome(BreakerOutcome::DeadlineClamped);
        }
        assert!(!b.is_open());
    }

    #[test]
    fn truncated_does_not_trip_by_default() {
        let b = CircuitBreaker::new(small_cfg());
        for _ in 0..30 {
            b.try_acquire();
            b.record_outcome(BreakerOutcome::Truncated);
        }
        assert!(!b.is_open());
    }

    #[test]
    fn ignored_outcomes_do_not_advance_window() {
        let b = CircuitBreaker::new(small_cfg());
        // 4 deadline-clamped (ignored) + 1 fail should NOT trip:
        // ignored don't advance window so call_count = 1.
        for _ in 0..4 {
            b.try_acquire();
            b.record_outcome(BreakerOutcome::DeadlineClamped);
        }
        b.try_acquire();
        b.record_outcome(fail());
        assert!(!b.is_open());
    }

    #[test]
    fn ring_buffer_wraps_correctly() {
        let b = CircuitBreaker::new(small_cfg());
        // Fill window with successes
        for _ in 0..20 {
            b.try_acquire();
            b.record_outcome(ok());
        }
        // Add 5 failures — they wrap in but don't reach 50% (5/10 = 50%, just at threshold)
        for _ in 0..5 {
            b.try_acquire();
            b.record_outcome(fail());
        }
        // 5 fail + 5 ok = 50% exactly ≥ 50% → trip.
        assert!(b.is_open());
    }

    #[test]
    fn old_failures_age_out() {
        // Use a wider window where failures stay safely below threshold
        // and then get pushed out by a tide of successes.
        let mut cfg = small_cfg();
        cfg.window_size = 20;
        cfg.min_calls = 10;
        let b = CircuitBreaker::new(cfg);
        // Pre-fill with successes so we never cross threshold mid-stream.
        for _ in 0..10 {
            b.try_acquire();
            b.record_outcome(ok());
        }
        // Add 3 failures: 3/13 ≈ 23% < 50% → no trip.
        for _ in 0..3 {
            b.try_acquire();
            b.record_outcome(fail());
        }
        assert!(!b.is_open());
        // 20 successes flood — failures evicted from the ring.
        for _ in 0..20 {
            b.try_acquire();
            b.record_outcome(ok());
        }
        assert!(!b.is_open());
    }

    #[test]
    fn half_open_close_on_majority_success() {
        let b = CircuitBreaker::new(small_cfg());
        for _ in 0..5 {
            b.try_acquire();
            b.record_outcome(fail());
        }
        assert!(b.is_open());
        std::thread::sleep(Duration::from_millis(25));

        // 3 probes → 2 success, 1 fail → 67% ≥ 60% → close.
        let p1 = b.try_acquire();
        let p2 = b.try_acquire();
        let p3 = b.try_acquire();
        assert_eq!(p1, Permit::Probe);
        assert_eq!(p2, Permit::Probe);
        assert_eq!(p3, Permit::Probe);
        // No more probes.
        assert_eq!(b.try_acquire(), Permit::Rejected);
        b.record_outcome(ok());
        b.record_outcome(ok());
        b.record_outcome(fail());
        assert!(!b.is_open());
    }

    #[test]
    fn half_open_reopen_on_minority_success() {
        let b = CircuitBreaker::new(small_cfg());
        for _ in 0..5 {
            b.try_acquire();
            b.record_outcome(fail());
        }
        std::thread::sleep(Duration::from_millis(25));

        // 3 probes → 1 success, 2 fail → 33% < 60% → reopen.
        b.try_acquire();
        b.try_acquire();
        b.try_acquire();
        b.record_outcome(ok());
        b.record_outcome(fail());
        b.record_outcome(fail());
        assert!(b.is_open());
    }

    #[test]
    fn cooldown_grows_with_ejection_count() {
        let mut cfg = small_cfg();
        cfg.base_cooldown = Duration::from_millis(20);
        cfg.max_cooldown = Duration::from_millis(200);
        let b = CircuitBreaker::new(cfg);
        // First trip
        for _ in 0..5 {
            b.try_acquire();
            b.record_outcome(fail());
        }
        let s1 = b.snapshot();
        assert_eq!(s1.ejection_count, 1);
        // Wait, half-open, fail probes, reopen → ejection 2
        std::thread::sleep(Duration::from_millis(25));
        b.try_acquire();
        b.try_acquire();
        b.try_acquire();
        b.record_outcome(fail());
        b.record_outcome(fail());
        b.record_outcome(fail());
        let s2 = b.snapshot();
        assert_eq!(s2.ejection_count, 2);
        // Cooldown should be ~40ms now (base*2)
        assert!(s2.opens_in_seconds.is_some());
    }

    #[test]
    fn cancel_probe_decrements_admitted() {
        let b = CircuitBreaker::new(small_cfg());
        for _ in 0..5 {
            b.try_acquire();
            b.record_outcome(fail());
        }
        std::thread::sleep(Duration::from_millis(25));
        let _ = b.try_acquire(); // admitted=1
        let _ = b.try_acquire(); // admitted=2
        let _ = b.try_acquire(); // admitted=3
        assert_eq!(b.try_acquire(), Permit::Rejected); // capped
        b.cancel_probe(); // admitted=2
        // Now another probe is allowed.
        assert_eq!(b.try_acquire(), Permit::Probe);
    }

    #[test]
    fn reset_clears_state() {
        let b = CircuitBreaker::new(small_cfg());
        for _ in 0..5 {
            b.try_acquire();
            b.record_outcome(fail());
        }
        assert!(b.is_open());
        b.reset();
        assert!(!b.is_open());
        assert_eq!(b.snapshot().ejection_count, 0);
        assert_eq!(b.snapshot().window_call_count, 0);
    }

    #[test]
    fn classify_outcome_deadline_clamped() {
        let ctx = AttemptContext::capture(Duration::from_millis(500), Duration::from_millis(2500));
        let outcome = classify_outcome(false, false, true, &ctx);
        assert_eq!(outcome, BreakerOutcome::DeadlineClamped);
    }

    #[test]
    fn classify_outcome_tier_timeout_with_full_budget() {
        let ctx = AttemptContext::capture(Duration::from_millis(8000), Duration::from_millis(2500));
        let outcome = classify_outcome(false, false, true, &ctx);
        assert_eq!(outcome, BreakerOutcome::TierTimeout);
    }

    #[test]
    fn classify_outcome_truncated_success() {
        let ctx = AttemptContext::capture(Duration::from_millis(8000), Duration::from_millis(2500));
        let outcome = classify_outcome(true, true, false, &ctx);
        assert_eq!(outcome, BreakerOutcome::Truncated);
    }
}
