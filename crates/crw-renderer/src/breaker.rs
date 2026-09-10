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
//! `DeadlineClamped` and `SiteBlocked` are observed via
//! `crw_breaker_ignored_total` only. `Truncated` is configurable (default
//! ignored — chrome partial-DOM is a feature, not a tier failure).

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
            // Loosened May 2026 after the 1000-URL bench traced 117/214 failures
            // to false-trips in the global breaker. The bench-natural failure
            // rate (anti-bot blocks + 4xx/5xx + soft fails + thin renders) sits
            // in the 30-45% band, which the prior 0.55 / N=50 config crossed
            // 5-6 times per run and shed ~12% of throughput. Threshold 0.80 +
            // window 100 + min_calls 50 only trip when something is genuinely
            // broken (e.g. LP segfault loop, chrome disconnect storm), not
            // when a bursty cluster of hard URLs hits the queue. Cooldown
            // dropped 10s→5s so a transient blip recovers within one bench
            // window-roll instead of compounding to 30s+ via ejection_count.
            window_size: 100,
            min_calls: 50,
            failure_rate_threshold: 0.80,
            base_cooldown: Duration::from_secs(5),
            max_cooldown: Duration::from_secs(60),
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
    /// The origin refused *us*, not this tier: an anti-bot wall, a vendor
    /// block, a Cloudflare interstitial, or a block status. Every tier would
    /// see the same thing, so it says nothing about tier health. Observed via
    /// `crw_breaker_ignored_total` only, exactly like `DeadlineClamped`.
    ///
    /// Recording it as `RenderError` is what let a blocked host trip the
    /// per-host breaker for lightpanda *and* chrome, leaving `fetch_with_js`
    /// with no renderer and stranding the one tier (residential egress) that
    /// could have served the page.
    SiteBlocked,
}

impl BreakerOutcome {
    fn is_failure(&self, count_truncated_as_failure: bool) -> bool {
        match self {
            BreakerOutcome::Success => false,
            BreakerOutcome::Truncated => count_truncated_as_failure,
            BreakerOutcome::DeadlineClamped => false,
            BreakerOutcome::SiteBlocked => false,
            BreakerOutcome::TierTimeout
            | BreakerOutcome::ConnectionError
            | BreakerOutcome::RenderError => true,
        }
    }

    /// True if this outcome should advance the failure window at all.
    /// `DeadlineClamped` and `SiteBlocked` are fully ignored (only counted in
    /// observability). `Truncated` is conditionally ignored.
    ///
    /// NOTE: this match and `ignored_reason` below both end in a wildcard, so
    /// adding a variant and updating only `is_failure` compiles and silently
    /// keeps advancing the window. Any new variant must be considered here too.
    fn advances_window(&self, count_truncated_as_failure: bool) -> bool {
        match self {
            BreakerOutcome::DeadlineClamped | BreakerOutcome::SiteBlocked => false,
            BreakerOutcome::Truncated => count_truncated_as_failure,
            _ => true,
        }
    }

    pub fn ignored_reason(&self) -> Option<&'static str> {
        match self {
            BreakerOutcome::DeadlineClamped => Some("deadline_clamped"),
            BreakerOutcome::SiteBlocked => Some("site_blocked"),
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
///
/// `site_blocked` wins over every failure class, including a timeout: an origin
/// that walls us can also be slow about it, and the wall is still not this
/// tier's fault. Callers must compute it fresh per attempt — never from a
/// cumulative "did anything in this request see a block" flag, or one tier's
/// block would mask the next tier's genuine render failure.
pub fn classify_outcome(
    success: bool,
    is_truncated: bool,
    error_was_timeout: bool,
    site_blocked: bool,
    ctx: &AttemptContext,
) -> BreakerOutcome {
    if !success && site_blocked {
        return BreakerOutcome::SiteBlocked;
    }
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
    global: Arc<[(RendererKind, Arc<CircuitBreaker>); 6]>,
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
            (
                RendererKind::ChromeProxy,
                Arc::new(CircuitBreaker::new(config)),
            ),
            // Unconditional (like every other kind). Harmless dead capacity in
            // lean builds — gating the fixed-size array on a feature would
            // complicate its type for no behavioural gain.
            (
                RendererKind::Camoufox,
                Arc::new(CircuitBreaker::new(config)),
            ),
            // Unconditional (like every other kind). Harmless dead capacity in
            // lean builds; a cloak failure trips only the Cloak breaker.
            (RendererKind::Cloak, Arc::new(CircuitBreaker::new(config))),
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
        // ImpersonatedHttp is a RendererKind but deliberately NOT in this
        // registry: it changes the TLS fingerprint, not the egress IP or the
        // browser, so a global breaker on it would trip the wrong thing. It
        // never reaches here today (it is absent from `js_renderers`, the only
        // source this registry is keyed from). Reaching this arm therefore
        // means a caller started recording breaker outcomes for a tier that
        // has no breaker, which is a design violation worth failing loudly on.
        unreachable!(
            "no global breaker for {renderer:?}: this registry covers the ladder tiers \
             (Http | Lightpanda | Chrome | ChromeProxy | Camoufox | Cloak) only"
        )
    }

    pub async fn host_for(&self, host: &str, renderer: RendererKind) -> Arc<CircuitBreaker> {
        let key = (normalize_host(host), renderer);
        let cfg = self.config;
        self.host
            .get_with(key, async move { Arc::new(CircuitBreaker::new(cfg)) })
            .await
    }

    /// Acquire a permit consulting the host tier only, ignoring global
    /// breaker state. Used by the "leak" fallback: when every renderer
    /// has been skipped because the global breaker is open but the
    /// per-host breaker is closed, give one renderer a single shot
    /// rather than fail the request outright. The host breaker still
    /// guards against repeatedly hammering a host that's known broken.
    pub async fn try_acquire_host_only(&self, host: &str, renderer: RendererKind) -> Permit {
        self.host_for(host, renderer).await.try_acquire()
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

    /// Emit a single source-of-truth event when a tier transitions to Open:
    /// bumps the Prometheus counter and logs a structured tracing line.
    /// Centralizes the two emissions so they never drift out of sync.
    fn emit_breaker_opened(&self, renderer: RendererKind, scope: &'static str, host: &str) {
        metrics()
            .circuit_breaker_open_total
            .with_label_values(&[renderer.as_str(), scope])
            .inc();
        tracing::info!(
            renderer = renderer.as_str(),
            scope,
            host = host,
            "breaker_opened"
        );
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
            self.emit_breaker_opened(renderer, "global", host);
        }
        if h_tripped {
            self.emit_breaker_opened(renderer, "host", host);
        }
    }

    /// Record outcomes independently to global vs host tiers. Use when a
    /// failure is page-/content-specific (anti-bot, thin SPA shell,
    /// target-host network error) and should not poison global renderer
    /// availability. Pass `None` to skip recording on a tier.
    ///
    /// Background: the 1000-URL bench traced ~12% of failures to false
    /// global trips — content-quality issues clustered onto a handful of
    /// hosts but the global window saw them as renderer-wide failures.
    /// Splitting accounting lets the host tier learn unsuitability while
    /// the global tier tracks only renderer-infrastructure health.
    pub async fn record_scoped_outcome(
        &self,
        host: &str,
        renderer: RendererKind,
        global_outcome: Option<BreakerOutcome>,
        host_outcome: Option<BreakerOutcome>,
    ) {
        // Emit the ignored-reason metric for whichever outcome is present, not
        // just the global one. The host-only callers (the leak-through arm) pass
        // `global_outcome: None`, so keeping this inside the global branch made
        // every outcome they record invisible on the dashboard — including the
        // `site_blocked` pressure this counter exists to surface. Prefer the
        // global label when both are present so a single call counts once.
        if let Some(reason) = global_outcome
            .or(host_outcome)
            .and_then(|o| o.ignored_reason())
        {
            metrics()
                .breaker_ignored_total
                .with_label_values(&[renderer.as_str(), reason])
                .inc();
        }
        // A tier we are NOT recording to must still have its probe released.
        //
        // `try_acquire` increments `admitted` when the breaker is HalfOpen, and
        // only `record_outcome` or `cancel_probe` ever moves that counter. A
        // caller that acquires a permit and then passes `None` for that tier
        // leaves the slot held forever: `admitted` saturates at `max_probes`
        // while `succeeded + failed` stays under it, `try_acquire` returns
        // Rejected for EVERY host, and only `lazy_evaluate`'s 30s eval_timeout
        // breaks the deadlock. Measured before this line existed: a half-open
        // global tier driven through three successes and two thin results went
        // `closed / Allowed` on production code and `half_open / Rejected` with
        // scoped recording.
        //
        // That is a recall path, not just latency: while the global tier is
        // Rejected the ladder skips it for every host, and the leak-through arm
        // only rescues a request whose `thin_result` is still None.
        //
        // `cancel_probe` is a no-op on Closed and Open, so this is free for the
        // host-only callers that were already correct.
        match global_outcome {
            Some(outcome) => {
                let g_tripped = self.global_for(renderer).record_outcome(outcome);
                if g_tripped {
                    self.emit_breaker_opened(renderer, "global", host);
                }
            }
            None => self.global_for(renderer).cancel_probe(),
        }
        match host_outcome {
            Some(outcome) => {
                let h_tripped = self.host_for(host, renderer).await.record_outcome(outcome);
                if h_tripped {
                    self.emit_breaker_opened(renderer, "host", host);
                }
            }
            None => self.host_for(host, renderer).await.cancel_probe(),
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
        let outcome = classify_outcome(false, false, true, false, &ctx);
        assert_eq!(outcome, BreakerOutcome::DeadlineClamped);
    }

    #[test]
    fn classify_outcome_tier_timeout_with_full_budget() {
        let ctx = AttemptContext::capture(Duration::from_millis(8000), Duration::from_millis(2500));
        let outcome = classify_outcome(false, false, true, false, &ctx);
        assert_eq!(outcome, BreakerOutcome::TierTimeout);
    }

    #[test]
    fn classify_outcome_truncated_success() {
        let ctx = AttemptContext::capture(Duration::from_millis(8000), Duration::from_millis(2500));
        let outcome = classify_outcome(true, true, false, false, &ctx);
        assert_eq!(outcome, BreakerOutcome::Truncated);
    }

    /// The bug this exists for: an anti-bot wall is a property of the ORIGIN, so
    /// every tier egressing from this IP sees it. Counting it as a tier failure
    /// tripped the per-host breaker for lightpanda AND chrome, which left
    /// `fetch_with_js` with no renderer to run and stranded the residential tier
    /// that could have served the page.
    #[test]
    fn site_blocks_never_open_the_breaker() {
        let b = CircuitBreaker::new(small_cfg());
        // Far more than `min_calls`, all blocks.
        for _ in 0..200 {
            b.record_outcome(BreakerOutcome::SiteBlocked);
        }
        assert!(
            !b.is_open(),
            "a walled origin must not be recorded as renderer ill-health"
        );
        assert_eq!(
            b.snapshot().window_call_count,
            0,
            "SiteBlocked must not advance the window at all"
        );
    }

    /// A blocked host must not mask a genuinely broken tier: `SiteBlocked` is
    /// ignored, but real failures interleaved with it still count.
    #[test]
    fn site_blocks_do_not_mask_real_failures() {
        let b = CircuitBreaker::new(small_cfg());
        for _ in 0..50 {
            b.record_outcome(BreakerOutcome::SiteBlocked);
            b.record_outcome(fail());
        }
        assert!(
            b.is_open(),
            "interleaved genuine failures must still trip the breaker"
        );
    }

    #[test]
    fn site_blocked_is_observable_and_non_advancing() {
        assert_eq!(
            BreakerOutcome::SiteBlocked.ignored_reason(),
            Some("site_blocked"),
            "must be visible on the dashboard, not silently dropped"
        );
        assert!(!BreakerOutcome::SiteBlocked.is_failure(false));
        assert!(!BreakerOutcome::SiteBlocked.advances_window(false));
        // Guard against the wildcard trap: `advances_window` and
        // `ignored_reason` both end in `_ =>`, so a new variant that updates only
        // `is_failure` would compile and silently keep advancing the window.
        assert!(!BreakerOutcome::DeadlineClamped.advances_window(false));
    }

    /// `site_blocked` outranks a timeout: an origin that walls us can also be
    /// slow about it, and the wall is still not the tier's fault.
    #[test]
    fn classify_outcome_site_blocked_beats_timeout() {
        let ctx = AttemptContext::capture(Duration::from_millis(8000), Duration::from_millis(2500));
        assert_eq!(
            classify_outcome(false, false, true, true, &ctx),
            BreakerOutcome::SiteBlocked
        );
        // …but a SUCCESS is still a success, block flag or not.
        assert_eq!(
            classify_outcome(true, false, false, true, &ctx),
            BreakerOutcome::Success
        );
    }

    /// Documents a real side effect of non-advancing outcomes rather than
    /// leaving it to be discovered later: because the HalfOpen arm returns the
    /// admitted slot, a stream of `SiteBlocked` never reaches `max_probes`, so
    /// HalfOpen cannot be decided by probe count and exits only via
    /// `eval_timeout`. Until then it admits unbounded probes. Accepted because
    /// per-host in-flight work is separately capped by `host_limiter`, and the
    /// alternative (counting blocks) is the bug this whole change removes.
    #[test]
    fn site_blocked_probes_are_returned_not_counted() {
        let cfg = BreakerConfig {
            max_probes: 2,
            ..small_cfg()
        };
        let b = CircuitBreaker::new(cfg);
        for _ in 0..cfg.min_calls * 2 {
            b.record_outcome(fail());
        }
        assert!(b.is_open());
        std::thread::sleep(cfg.base_cooldown + Duration::from_millis(50));
        // Now HalfOpen. Every probe reports a site block.
        for _ in 0..10 {
            assert_eq!(b.try_acquire(), Permit::Probe, "slot is always returned");
            b.record_outcome(BreakerOutcome::SiteBlocked);
        }
        assert!(
            !b.is_open(),
            "ignored outcomes must not re-open the breaker by themselves"
        );
    }

    #[test]
    fn registry_has_breaker_for_chrome_proxy() {
        let reg = BreakerRegistry::with_defaults();
        // Must not panic: global_for iterates a fixed 5-element array now.
        let _ = reg.global_for(RendererKind::ChromeProxy);
    }

    #[test]
    fn registry_has_breaker_for_camoufox() {
        let reg = BreakerRegistry::with_defaults();
        // Must not panic: the camoufox kind is a registered (5th) global tier.
        let _ = reg.global_for(RendererKind::Camoufox);
    }

    // ── Window (ring buffer) ──────────────────────────────────────────

    #[test]
    fn window_new_starts_all_empty() {
        let w = Window::new(5);
        assert_eq!(w.call_count(), 0);
        assert_eq!(w.failure_count(), 0);
        assert_eq!(w.failure_rate(), 0.0);
    }

    #[test]
    fn window_size_zero_is_clamped_to_one() {
        // `size.max(1)` — a zero-size ring must still be usable, not panic on push.
        let mut w = Window::new(0);
        w.push(WindowSlot::Success);
        assert_eq!(w.call_count(), 1);
        w.push(WindowSlot::Failure);
        // Ring of 1 — the failure overwrote the success.
        assert_eq!(w.call_count(), 1);
        assert_eq!(w.failure_count(), 1);
    }

    #[test]
    fn window_push_wraps_cursor_at_boundary() {
        let mut w = Window::new(2);
        w.push(WindowSlot::Failure);
        w.push(WindowSlot::Failure);
        // Third push wraps to index 0, overwriting the first failure with success.
        w.push(WindowSlot::Success);
        assert_eq!(w.call_count(), 2);
        assert_eq!(w.failure_count(), 1);
    }

    #[test]
    fn window_failure_rate_all_success_is_zero() {
        let mut w = Window::new(4);
        for _ in 0..4 {
            w.push(WindowSlot::Success);
        }
        assert_eq!(w.failure_rate(), 0.0);
    }

    #[test]
    fn window_failure_rate_all_failure_is_one() {
        let mut w = Window::new(4);
        for _ in 0..4 {
            w.push(WindowSlot::Failure);
        }
        assert_eq!(w.failure_rate(), 1.0);
    }

    #[test]
    fn window_failure_rate_mixed_is_exact_fraction() {
        let mut w = Window::new(4);
        w.push(WindowSlot::Failure);
        w.push(WindowSlot::Success);
        w.push(WindowSlot::Success);
        w.push(WindowSlot::Success);
        assert_eq!(w.failure_rate(), 0.25);
    }

    #[test]
    fn window_call_count_ignores_unfilled_slots() {
        let mut w = Window::new(10);
        w.push(WindowSlot::Success);
        w.push(WindowSlot::Failure);
        // 8 slots still Empty — must not count toward call_count.
        assert_eq!(w.call_count(), 2);
    }

    #[test]
    fn window_clear_resets_slots_and_cursor() {
        let mut w = Window::new(3);
        w.push(WindowSlot::Failure);
        w.push(WindowSlot::Failure);
        w.clear();
        assert_eq!(w.call_count(), 0);
        assert_eq!(w.failure_rate(), 0.0);
        // Cursor reset to 0: the next push lands at index 0, not index 2.
        w.push(WindowSlot::Success);
        assert_eq!(w.ring[0], WindowSlot::Success);
    }

    // ── BreakerOutcome::is_failure matrix ────────────────────────────

    #[test]
    fn is_failure_success_is_never_a_failure() {
        assert!(!BreakerOutcome::Success.is_failure(false));
        assert!(!BreakerOutcome::Success.is_failure(true));
    }

    #[test]
    fn is_failure_truncated_depends_on_config() {
        assert!(!BreakerOutcome::Truncated.is_failure(false));
        assert!(BreakerOutcome::Truncated.is_failure(true));
    }

    #[test]
    fn is_failure_deadline_clamped_is_never_a_failure() {
        assert!(!BreakerOutcome::DeadlineClamped.is_failure(false));
        assert!(!BreakerOutcome::DeadlineClamped.is_failure(true));
    }

    #[test]
    fn is_failure_site_blocked_is_never_a_failure() {
        assert!(!BreakerOutcome::SiteBlocked.is_failure(false));
        assert!(!BreakerOutcome::SiteBlocked.is_failure(true));
    }

    #[test]
    fn is_failure_tier_timeout_connection_render_are_always_failures() {
        for outcome in [
            BreakerOutcome::TierTimeout,
            BreakerOutcome::ConnectionError,
            BreakerOutcome::RenderError,
        ] {
            assert!(outcome.is_failure(false));
            assert!(outcome.is_failure(true));
        }
    }

    // ── BreakerOutcome::advances_window matrix ───────────────────────

    #[test]
    fn advances_window_success_always_advances() {
        assert!(BreakerOutcome::Success.advances_window(false));
        assert!(BreakerOutcome::Success.advances_window(true));
    }

    #[test]
    fn advances_window_deadline_clamped_never_advances() {
        assert!(!BreakerOutcome::DeadlineClamped.advances_window(false));
        assert!(!BreakerOutcome::DeadlineClamped.advances_window(true));
    }

    #[test]
    fn advances_window_site_blocked_never_advances() {
        assert!(!BreakerOutcome::SiteBlocked.advances_window(false));
        assert!(!BreakerOutcome::SiteBlocked.advances_window(true));
    }

    #[test]
    fn advances_window_truncated_depends_on_config() {
        assert!(!BreakerOutcome::Truncated.advances_window(false));
        assert!(BreakerOutcome::Truncated.advances_window(true));
    }

    #[test]
    fn advances_window_tier_timeout_connection_render_always_advance() {
        for outcome in [
            BreakerOutcome::TierTimeout,
            BreakerOutcome::ConnectionError,
            BreakerOutcome::RenderError,
        ] {
            assert!(outcome.advances_window(false));
            assert!(outcome.advances_window(true));
        }
    }

    // ── BreakerOutcome::ignored_reason matrix ────────────────────────

    #[test]
    fn ignored_reason_deadline_clamped() {
        assert_eq!(
            BreakerOutcome::DeadlineClamped.ignored_reason(),
            Some("deadline_clamped")
        );
    }

    #[test]
    fn ignored_reason_truncated() {
        assert_eq!(
            BreakerOutcome::Truncated.ignored_reason(),
            Some("truncated")
        );
    }

    #[test]
    fn ignored_reason_none_for_advancing_outcomes() {
        for outcome in [
            BreakerOutcome::Success,
            BreakerOutcome::TierTimeout,
            BreakerOutcome::ConnectionError,
            BreakerOutcome::RenderError,
        ] {
            assert_eq!(outcome.ignored_reason(), None);
        }
    }

    // ── AttemptContext::capture ──────────────────────────────────────

    #[test]
    fn attempt_context_clamped_when_budget_exceeds_remaining() {
        let ctx = AttemptContext::capture(Duration::from_millis(100), Duration::from_millis(500));
        assert!(ctx.was_clamped_by_deadline);
    }

    #[test]
    fn attempt_context_not_clamped_when_remaining_exceeds_budget() {
        let ctx = AttemptContext::capture(Duration::from_millis(500), Duration::from_millis(100));
        assert!(!ctx.was_clamped_by_deadline);
    }

    #[test]
    fn attempt_context_boundary_equal_is_not_clamped() {
        // `>` not `>=`: exactly-equal budget/remaining is NOT clamped.
        let ctx = AttemptContext::capture(Duration::from_millis(200), Duration::from_millis(200));
        assert!(!ctx.was_clamped_by_deadline);
    }

    #[test]
    fn attempt_context_zero_remaining_is_clamped_by_any_positive_budget() {
        let ctx = AttemptContext::capture(Duration::ZERO, Duration::from_millis(1));
        assert!(ctx.was_clamped_by_deadline);
    }

    #[test]
    fn attempt_context_zero_budget_is_never_clamped() {
        let ctx = AttemptContext::capture(Duration::from_millis(500), Duration::ZERO);
        assert!(!ctx.was_clamped_by_deadline);
    }

    #[test]
    fn attempt_context_preserves_captured_fields() {
        let ctx = AttemptContext::capture(Duration::from_millis(42), Duration::from_millis(7));
        assert_eq!(ctx.remaining_at_start, Duration::from_millis(42));
        assert_eq!(ctx.tier_budget, Duration::from_millis(7));
    }

    // ── classify_outcome matrix ───────────────────────────────────────

    #[test]
    fn classify_success_untrucated() {
        let ctx = AttemptContext::capture(Duration::from_secs(5), Duration::from_secs(1));
        assert_eq!(
            classify_outcome(true, false, false, false, &ctx),
            BreakerOutcome::Success
        );
    }

    #[test]
    fn classify_success_truncated() {
        let ctx = AttemptContext::capture(Duration::from_secs(5), Duration::from_secs(1));
        assert_eq!(
            classify_outcome(true, true, false, false, &ctx),
            BreakerOutcome::Truncated
        );
    }

    #[test]
    fn classify_failure_not_timeout_is_render_error() {
        let ctx = AttemptContext::capture(Duration::from_secs(5), Duration::from_secs(1));
        assert_eq!(
            classify_outcome(false, false, false, false, &ctx),
            BreakerOutcome::RenderError
        );
    }

    #[test]
    fn classify_failure_timeout_full_budget_is_tier_timeout() {
        let ctx = AttemptContext::capture(Duration::from_secs(5), Duration::from_secs(1));
        assert_eq!(
            classify_outcome(false, false, true, false, &ctx),
            BreakerOutcome::TierTimeout
        );
    }

    #[test]
    fn classify_failure_timeout_clamped_budget_is_deadline_clamped() {
        let ctx = AttemptContext::capture(Duration::from_millis(1), Duration::from_secs(5));
        assert_eq!(
            classify_outcome(false, false, true, false, &ctx),
            BreakerOutcome::DeadlineClamped
        );
    }

    #[test]
    fn classify_site_blocked_wins_over_plain_render_error() {
        let ctx = AttemptContext::capture(Duration::from_secs(5), Duration::from_secs(1));
        assert_eq!(
            classify_outcome(false, false, false, true, &ctx),
            BreakerOutcome::SiteBlocked
        );
    }

    #[test]
    fn classify_site_blocked_flag_ignored_on_success() {
        // Already covered by `classify_outcome_site_blocked_beats_timeout` for
        // the timeout branch; this covers the plain-failure branch.
        let ctx = AttemptContext::capture(Duration::from_secs(5), Duration::from_secs(1));
        assert_eq!(
            classify_outcome(true, false, false, true, &ctx),
            BreakerOutcome::Success
        );
    }

    // ── min_calls boundary (N-1 / N / N+1) ────────────────────────────

    #[test]
    fn min_calls_boundary_n_minus_one_never_trips() {
        let mut cfg = small_cfg();
        cfg.min_calls = 5;
        cfg.failure_rate_threshold = 0.0; // any failure would trip if min_calls were met
        let b = CircuitBreaker::new(cfg);
        for _ in 0..4 {
            b.record_outcome(fail());
        }
        assert!(!b.is_open(), "4 calls < min_calls=5 must never trip");
    }

    #[test]
    fn min_calls_boundary_exact_n_trips() {
        let mut cfg = small_cfg();
        cfg.min_calls = 5;
        cfg.failure_rate_threshold = 0.0;
        let b = CircuitBreaker::new(cfg);
        for _ in 0..5 {
            b.record_outcome(fail());
        }
        assert!(
            b.is_open(),
            "exactly min_calls=5 with 100% failure must trip"
        );
    }

    #[test]
    fn min_calls_boundary_n_plus_one_trips() {
        let mut cfg = small_cfg();
        cfg.min_calls = 5;
        cfg.failure_rate_threshold = 0.0;
        cfg.window_size = 10;
        let b = CircuitBreaker::new(cfg);
        for _ in 0..6 {
            b.record_outcome(fail());
        }
        assert!(b.is_open());
    }

    // ── failure_rate_threshold boundary ───────────────────────────────

    #[test]
    fn failure_rate_boundary_just_below_threshold_no_trip() {
        let mut cfg = small_cfg();
        cfg.window_size = 100;
        cfg.min_calls = 100;
        cfg.failure_rate_threshold = 0.5;
        let b = CircuitBreaker::new(cfg);
        for _ in 0..49 {
            b.record_outcome(fail());
        }
        for _ in 0..51 {
            b.record_outcome(ok());
        }
        // 49/100 = 49% < 50%.
        assert!(!b.is_open());
    }

    #[test]
    fn failure_rate_boundary_exactly_at_threshold_trips() {
        let mut cfg = small_cfg();
        cfg.window_size = 100;
        cfg.min_calls = 100;
        cfg.failure_rate_threshold = 0.5;
        let b = CircuitBreaker::new(cfg);
        for _ in 0..50 {
            b.record_outcome(fail());
        }
        for _ in 0..50 {
            b.record_outcome(ok());
        }
        // 50/100 = 50% >= 50% (inclusive) → trip.
        assert!(b.is_open());
    }

    #[test]
    fn failure_rate_boundary_just_above_threshold_trips() {
        let mut cfg = small_cfg();
        cfg.window_size = 100;
        cfg.min_calls = 100;
        cfg.failure_rate_threshold = 0.5;
        let b = CircuitBreaker::new(cfg);
        for _ in 0..51 {
            b.record_outcome(fail());
        }
        for _ in 0..49 {
            b.record_outcome(ok());
        }
        assert!(b.is_open());
    }

    // ── max_probes boundary ───────────────────────────────────────────

    #[test]
    fn max_probes_admits_exactly_configured_count() {
        let mut cfg = small_cfg();
        cfg.max_probes = 4;
        let b = CircuitBreaker::new(cfg);
        for _ in 0..5 {
            b.record_outcome(fail());
        }
        assert!(b.is_open());
        std::thread::sleep(cfg.base_cooldown + Duration::from_millis(10));
        for _ in 0..4 {
            assert_eq!(b.try_acquire(), Permit::Probe);
        }
    }

    #[test]
    fn max_probes_n_plus_one_is_rejected() {
        let mut cfg = small_cfg();
        cfg.max_probes = 4;
        let b = CircuitBreaker::new(cfg);
        for _ in 0..5 {
            b.record_outcome(fail());
        }
        std::thread::sleep(cfg.base_cooldown + Duration::from_millis(10));
        for _ in 0..4 {
            b.try_acquire();
        }
        assert_eq!(b.try_acquire(), Permit::Rejected);
    }

    #[test]
    fn max_probes_of_one_admits_a_single_probe() {
        let mut cfg = small_cfg();
        cfg.max_probes = 1;
        let b = CircuitBreaker::new(cfg);
        for _ in 0..5 {
            b.record_outcome(fail());
        }
        std::thread::sleep(cfg.base_cooldown + Duration::from_millis(10));
        assert_eq!(b.try_acquire(), Permit::Probe);
        assert_eq!(b.try_acquire(), Permit::Rejected);
    }

    // ── half_open_success_rate boundary ───────────────────────────────

    #[test]
    fn half_open_success_rate_exactly_at_threshold_closes() {
        let mut cfg = small_cfg();
        cfg.max_probes = 5;
        cfg.half_open_success_rate = 0.6; // exactly 3/5
        let b = CircuitBreaker::new(cfg);
        for _ in 0..5 {
            b.record_outcome(fail());
        }
        std::thread::sleep(cfg.base_cooldown + Duration::from_millis(10));
        for _ in 0..5 {
            b.try_acquire();
        }
        b.record_outcome(ok());
        b.record_outcome(ok());
        b.record_outcome(ok());
        b.record_outcome(fail());
        b.record_outcome(fail());
        assert!(!b.is_open(), "3/5 = 60% >= 60% must close");
    }

    #[test]
    fn half_open_success_rate_one_below_threshold_reopens() {
        let mut cfg = small_cfg();
        cfg.max_probes = 5;
        cfg.half_open_success_rate = 0.6; // 3/5 required, give only 2/5
        let b = CircuitBreaker::new(cfg);
        for _ in 0..5 {
            b.record_outcome(fail());
        }
        std::thread::sleep(cfg.base_cooldown + Duration::from_millis(10));
        for _ in 0..5 {
            b.try_acquire();
        }
        b.record_outcome(ok());
        b.record_outcome(ok());
        b.record_outcome(fail());
        b.record_outcome(fail());
        b.record_outcome(fail());
        assert!(b.is_open(), "2/5 = 40% < 60% must reopen");
    }

    // ── eval_timeout forced half-open decision ────────────────────────

    #[test]
    fn eval_timeout_with_no_successes_reopens() {
        let mut cfg = small_cfg();
        cfg.max_probes = 3;
        cfg.eval_timeout = Duration::from_millis(30);
        let b = CircuitBreaker::new(cfg);
        for _ in 0..5 {
            b.record_outcome(fail());
        }
        std::thread::sleep(cfg.base_cooldown + Duration::from_millis(10));
        // Admit only 1 of 3 probes and fail it, then let eval_timeout elapse
        // without ever reaching max_probes.
        assert_eq!(b.try_acquire(), Permit::Probe);
        b.record_outcome(fail());
        std::thread::sleep(cfg.eval_timeout + Duration::from_millis(20));
        assert!(
            b.is_open(),
            "eval_timeout with zero recorded successes must force-reopen"
        );
    }

    #[test]
    fn eval_timeout_with_partial_success_closes() {
        let mut cfg = small_cfg();
        cfg.max_probes = 3;
        cfg.eval_timeout = Duration::from_millis(30);
        let b = CircuitBreaker::new(cfg);
        for _ in 0..5 {
            b.record_outcome(fail());
        }
        std::thread::sleep(cfg.base_cooldown + Duration::from_millis(10));
        assert_eq!(b.try_acquire(), Permit::Probe);
        b.record_outcome(ok());
        std::thread::sleep(cfg.eval_timeout + Duration::from_millis(20));
        assert!(
            !b.is_open(),
            "eval_timeout with at least one success must force-close"
        );
    }

    #[test]
    fn eval_timeout_with_zero_probes_admitted_reopens() {
        let mut cfg = small_cfg();
        cfg.max_probes = 3;
        cfg.eval_timeout = Duration::from_millis(30);
        let b = CircuitBreaker::new(cfg);
        for _ in 0..5 {
            b.record_outcome(fail());
        }
        std::thread::sleep(cfg.base_cooldown + Duration::from_millis(10));
        // Enter HalfOpen (lazily) without admitting any probe at all.
        assert!(!b.is_open());
        std::thread::sleep(cfg.eval_timeout + Duration::from_millis(20));
        assert!(
            b.is_open(),
            "nobody ever probed → no evidence of recovery → reopen"
        );
    }

    // ── ejection_reset_after_closed ────────────────────────────────────

    #[test]
    fn ejection_count_resets_after_sustained_closed_period() {
        let mut cfg = small_cfg();
        cfg.max_probes = 1;
        cfg.half_open_success_rate = 0.0; // any single probe outcome closes
        cfg.ejection_reset_after_closed = Duration::from_millis(30);
        let b = CircuitBreaker::new(cfg);
        for _ in 0..5 {
            b.record_outcome(fail());
        }
        assert_eq!(b.snapshot().ejection_count, 1);
        std::thread::sleep(cfg.base_cooldown + Duration::from_millis(10));
        b.try_acquire();
        b.record_outcome(ok()); // closes, ejection_count still 1
        assert_eq!(b.snapshot().ejection_count, 1);
        std::thread::sleep(cfg.ejection_reset_after_closed + Duration::from_millis(20));
        assert_eq!(
            b.snapshot().ejection_count,
            0,
            "sustained Closed period must reset ejection_count"
        );
    }

    #[test]
    fn ejection_count_not_reset_before_sustained_period_elapses() {
        let mut cfg = small_cfg();
        cfg.max_probes = 1;
        cfg.half_open_success_rate = 0.0;
        cfg.ejection_reset_after_closed = Duration::from_millis(500);
        let b = CircuitBreaker::new(cfg);
        for _ in 0..5 {
            b.record_outcome(fail());
        }
        std::thread::sleep(cfg.base_cooldown + Duration::from_millis(10));
        b.try_acquire();
        b.record_outcome(ok());
        assert_eq!(
            b.snapshot().ejection_count,
            1,
            "reset window has not elapsed yet — ejection_count must persist"
        );
    }

    // ── cooldown growth ─────────────────────────────────────────────────

    #[test]
    fn cooldown_capped_at_max_after_many_ejections() {
        let mut cfg = small_cfg();
        cfg.base_cooldown = Duration::from_millis(20);
        cfg.max_cooldown = Duration::from_millis(45);
        cfg.max_probes = 1;
        let b = CircuitBreaker::new(cfg);
        // Trip and reopen repeatedly to grow ejection_count well past the point
        // where base_cooldown * ejection_count would exceed max_cooldown.
        for _ in 0..5 {
            b.record_outcome(fail());
        }
        for _ in 0..6 {
            std::thread::sleep(cfg.max_cooldown + Duration::from_millis(10));
            b.try_acquire();
            b.record_outcome(fail()); // fail the single probe → reopen, grow ejection_count
        }
        let snap = b.snapshot();
        assert!(snap.ejection_count >= 6);
        // opens_in_seconds is coarse (as_secs on a sub-second cooldown truncates
        // to 0), so assert indirectly: is_open must still be true immediately
        // after the last reopen (cooldown, whatever it is, has not elapsed yet).
        assert!(b.is_open());
    }

    // ── try_acquire / record_outcome state-machine edges ────────────────

    #[test]
    fn try_acquire_on_fresh_breaker_is_allowed() {
        let b = CircuitBreaker::new(small_cfg());
        assert_eq!(b.try_acquire(), Permit::Allowed);
    }

    #[test]
    fn try_acquire_while_open_is_rejected_repeatedly() {
        let b = CircuitBreaker::new(small_cfg());
        for _ in 0..5 {
            b.record_outcome(fail());
        }
        assert!(b.is_open());
        for _ in 0..5 {
            assert_eq!(b.try_acquire(), Permit::Rejected);
        }
    }

    #[test]
    fn record_outcome_returns_false_while_below_threshold() {
        let b = CircuitBreaker::new(small_cfg());
        for _ in 0..3 {
            assert!(!b.record_outcome(fail()));
        }
    }

    #[test]
    fn record_outcome_returns_true_exactly_on_the_tripping_call() {
        let b = CircuitBreaker::new(small_cfg());
        let mut results = Vec::new();
        for _ in 0..5 {
            results.push(b.record_outcome(fail()));
        }
        assert_eq!(results, vec![false, false, false, false, true]);
    }

    #[test]
    fn record_outcome_while_open_is_a_noop_and_returns_false() {
        let b = CircuitBreaker::new(small_cfg());
        for _ in 0..5 {
            b.record_outcome(fail());
        }
        assert!(b.is_open());
        // Recording further outcomes while genuinely Open (not yet past
        // cooldown) must not panic and must return false.
        assert!(!b.record_outcome(ok()));
        assert!(!b.record_outcome(fail()));
    }

    #[test]
    fn cancel_probe_on_closed_state_is_a_harmless_noop() {
        let b = CircuitBreaker::new(small_cfg());
        b.cancel_probe(); // must not panic
        assert_eq!(b.try_acquire(), Permit::Allowed);
    }

    #[test]
    fn cancel_probe_on_open_state_is_a_harmless_noop() {
        let b = CircuitBreaker::new(small_cfg());
        for _ in 0..5 {
            b.record_outcome(fail());
        }
        assert!(b.is_open());
        b.cancel_probe(); // must not panic while Open
        assert!(b.is_open());
    }

    #[test]
    fn cancel_probe_saturating_sub_never_underflows() {
        let b = CircuitBreaker::new(small_cfg());
        for _ in 0..5 {
            b.record_outcome(fail());
        }
        std::thread::sleep(Duration::from_millis(25));
        // No probes admitted yet — cancel repeatedly must not panic or wrap.
        for _ in 0..10 {
            b.cancel_probe();
        }
        // Full probe quota must still be available afterward.
        for _ in 0..3 {
            assert_eq!(b.try_acquire(), Permit::Probe);
        }
    }

    #[test]
    fn half_open_ignored_outcome_frees_slot_without_deciding() {
        let mut cfg = small_cfg();
        cfg.max_probes = 2;
        let b = CircuitBreaker::new(cfg);
        for _ in 0..5 {
            b.record_outcome(fail());
        }
        std::thread::sleep(cfg.base_cooldown + Duration::from_millis(10));
        assert_eq!(b.try_acquire(), Permit::Probe);
        // DeadlineClamped is ignored: frees the admitted slot without deciding.
        b.record_outcome(BreakerOutcome::DeadlineClamped);
        // Full quota must still be available — the ignored probe didn't consume it.
        assert_eq!(b.try_acquire(), Permit::Probe);
        assert_eq!(b.try_acquire(), Permit::Probe);
        assert_eq!(b.try_acquire(), Permit::Rejected);
    }

    #[test]
    fn is_open_is_false_during_half_open() {
        let b = CircuitBreaker::new(small_cfg());
        for _ in 0..5 {
            b.record_outcome(fail());
        }
        assert!(b.is_open());
        std::thread::sleep(Duration::from_millis(25));
        // is_open() itself triggers lazy_evaluate → transitions to HalfOpen.
        assert!(!b.is_open(), "HalfOpen must report is_open() == false");
    }

    #[test]
    fn is_open_lazily_transitions_open_to_half_open_without_try_acquire() {
        let b = CircuitBreaker::new(small_cfg());
        for _ in 0..5 {
            b.record_outcome(fail());
        }
        std::thread::sleep(Duration::from_millis(25));
        // Calling snapshot (not try_acquire) must still lazily evaluate.
        let snap = b.snapshot();
        assert_eq!(snap.state, "half_open");
    }

    // ── snapshot ──────────────────────────────────────────────────────

    #[test]
    fn snapshot_label_closed() {
        let b = CircuitBreaker::new(small_cfg());
        assert_eq!(b.snapshot().state, "closed");
        assert_eq!(b.snapshot().opens_in_seconds, None);
    }

    #[test]
    fn snapshot_label_open_has_opens_in_seconds() {
        let b = CircuitBreaker::new(small_cfg());
        for _ in 0..5 {
            b.record_outcome(fail());
        }
        let snap = b.snapshot();
        assert_eq!(snap.state, "open");
        assert!(snap.opens_in_seconds.is_some());
    }

    #[test]
    fn snapshot_label_half_open_has_no_opens_in_seconds() {
        let b = CircuitBreaker::new(small_cfg());
        for _ in 0..5 {
            b.record_outcome(fail());
        }
        std::thread::sleep(Duration::from_millis(25));
        let snap = b.snapshot();
        assert_eq!(snap.state, "half_open");
        assert_eq!(snap.opens_in_seconds, None);
    }

    #[test]
    fn snapshot_window_call_count_tracks_recorded_outcomes() {
        let b = CircuitBreaker::new(small_cfg());
        b.record_outcome(ok());
        b.record_outcome(ok());
        assert_eq!(b.snapshot().window_call_count, 2);
    }

    #[test]
    fn snapshot_window_failure_rate_matches_recorded_ratio() {
        let mut cfg = small_cfg();
        cfg.min_calls = 100; // stay Closed so the window keeps growing
        let b = CircuitBreaker::new(cfg);
        b.record_outcome(fail());
        b.record_outcome(ok());
        b.record_outcome(ok());
        b.record_outcome(ok());
        assert_eq!(b.snapshot().window_failure_rate, 0.25);
    }

    #[test]
    fn snapshot_ejection_count_increments_once_per_trip() {
        let mut cfg = small_cfg();
        cfg.max_probes = 1;
        let b = CircuitBreaker::new(cfg);
        assert_eq!(b.snapshot().ejection_count, 0);
        for _ in 0..5 {
            b.record_outcome(fail());
        }
        assert_eq!(b.snapshot().ejection_count, 1);
        std::thread::sleep(cfg.base_cooldown + Duration::from_millis(10));
        b.try_acquire();
        b.record_outcome(fail()); // reopen
        assert_eq!(b.snapshot().ejection_count, 2);
    }

    // ── reset ─────────────────────────────────────────────────────────

    #[test]
    fn reset_from_half_open_returns_to_closed() {
        let b = CircuitBreaker::new(small_cfg());
        for _ in 0..5 {
            b.record_outcome(fail());
        }
        std::thread::sleep(Duration::from_millis(25));
        assert_eq!(b.snapshot().state, "half_open");
        b.reset();
        assert_eq!(b.snapshot().state, "closed");
        assert_eq!(b.snapshot().ejection_count, 0);
    }

    #[test]
    fn reset_when_already_closed_is_safe_and_idempotent() {
        let b = CircuitBreaker::new(small_cfg());
        b.reset();
        b.reset();
        assert!(!b.is_open());
        assert_eq!(b.snapshot().window_call_count, 0);
    }

    #[test]
    fn reset_allows_immediate_allowed_permit_after_open() {
        let b = CircuitBreaker::new(small_cfg());
        for _ in 0..5 {
            b.record_outcome(fail());
        }
        assert_eq!(b.try_acquire(), Permit::Rejected);
        b.reset();
        assert_eq!(b.try_acquire(), Permit::Allowed);
    }

    // ── BreakerConfig::default ───────────────────────────────────────

    #[test]
    fn default_config_matches_documented_values() {
        let cfg = BreakerConfig::default();
        assert_eq!(cfg.window_size, 100);
        assert_eq!(cfg.min_calls, 50);
        assert_eq!(cfg.failure_rate_threshold, 0.80);
        assert_eq!(cfg.base_cooldown, Duration::from_secs(5));
        assert_eq!(cfg.max_cooldown, Duration::from_secs(60));
        assert_eq!(cfg.max_probes, 5);
        assert_eq!(cfg.half_open_success_rate, 0.60);
        assert_eq!(cfg.eval_timeout, Duration::from_secs(30));
        assert_eq!(cfg.ejection_reset_after_closed, Duration::from_secs(120));
        assert!(!cfg.count_truncated_as_failure);
    }

    #[test]
    fn default_breaker_registry_matches_with_defaults() {
        // `Default` and `with_defaults()` must agree — nothing should special-case
        // the trait impl to a divergent config.
        let via_default = BreakerRegistry::default();
        let via_ctor = BreakerRegistry::with_defaults();
        assert_eq!(
            via_default.config().window_size,
            via_ctor.config().window_size
        );
        assert_eq!(via_default.config().min_calls, via_ctor.config().min_calls);
    }

    // ── serde field naming (documents current wire shape) ─────────────

    #[test]
    fn breaker_status_serializes_with_snake_case_fields() {
        // NOTE: unlike the public v1/v2 API, `/admin/breakers` debug snapshots
        // are NOT camelCased — this test documents current behaviour, it is
        // not asserting that's correct API convention for a public surface.
        let status = BreakerStatus {
            renderer: "chrome".to_string(),
            state: "open".to_string(),
            opens_in_seconds: Some(5),
            ejection_count: 2,
            window_call_count: 10,
            window_failure_rate: 0.9,
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["window_call_count"], 10);
        assert_eq!(json["ejection_count"], 2);
        assert_eq!(json["opens_in_seconds"], 5);
    }

    #[test]
    fn breaker_status_serializes_none_opens_in_seconds_as_null() {
        let status = BreakerStatus {
            renderer: "http".to_string(),
            state: "closed".to_string(),
            opens_in_seconds: None,
            ejection_count: 0,
            window_call_count: 0,
            window_failure_rate: 0.0,
        };
        let json = serde_json::to_value(&status).unwrap();
        assert!(json["opens_in_seconds"].is_null());
    }

    #[test]
    fn registry_snapshot_serializes_nested_global_and_per_host() {
        let reg = BreakerRegistry::with_defaults();
        let snap = reg.snapshot();
        let json = serde_json::to_value(&snap).unwrap();
        assert!(json["global"].is_array());
        assert!(json["per_host"].is_array());
    }

    // ── BreakerRegistry ────────────────────────────────────────────────

    #[test]
    fn registry_global_for_returns_same_instance_for_same_kind() {
        let reg = BreakerRegistry::with_defaults();
        let a = reg.global_for(RendererKind::Chrome);
        let b = reg.global_for(RendererKind::Chrome);
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn registry_global_for_returns_distinct_instances_per_kind() {
        let reg = BreakerRegistry::with_defaults();
        let http = reg.global_for(RendererKind::Http);
        let chrome = reg.global_for(RendererKind::Chrome);
        assert!(!Arc::ptr_eq(&http, &chrome));
    }

    #[test]
    fn registry_global_for_covers_every_renderer_kind_without_panicking() {
        let reg = BreakerRegistry::with_defaults();
        for kind in [
            RendererKind::Http,
            RendererKind::Lightpanda,
            RendererKind::Chrome,
            RendererKind::ChromeProxy,
            RendererKind::Camoufox,
            RendererKind::Cloak,
        ] {
            let _ = reg.global_for(kind);
        }
    }

    #[test]
    fn registry_config_reflects_constructor_argument() {
        let cfg = BreakerConfig {
            min_calls: 7,
            ..BreakerConfig::default()
        };
        let reg = BreakerRegistry::new(cfg);
        assert_eq!(reg.config().min_calls, 7);
    }

    #[tokio::test]
    async fn registry_host_for_returns_same_breaker_for_same_host_and_renderer() {
        let reg = BreakerRegistry::with_defaults();
        let a = reg.host_for("example.com", RendererKind::Chrome).await;
        let b = reg.host_for("example.com", RendererKind::Chrome).await;
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[tokio::test]
    async fn registry_host_for_separates_different_hosts() {
        let reg = BreakerRegistry::with_defaults();
        let a = reg.host_for("a.com", RendererKind::Chrome).await;
        let b = reg.host_for("b.com", RendererKind::Chrome).await;
        assert!(!Arc::ptr_eq(&a, &b));
    }

    #[tokio::test]
    async fn registry_host_for_separates_different_renderers_on_same_host() {
        let reg = BreakerRegistry::with_defaults();
        let a = reg.host_for("example.com", RendererKind::Chrome).await;
        let b = reg.host_for("example.com", RendererKind::Http).await;
        assert!(!Arc::ptr_eq(&a, &b));
    }

    #[tokio::test]
    async fn registry_host_for_normalizes_www_and_subdomain_to_same_breaker() {
        let reg = BreakerRegistry::with_defaults();
        let bare = reg.host_for("example.com", RendererKind::Chrome).await;
        let www = reg.host_for("www.example.com", RendererKind::Chrome).await;
        let sub = reg.host_for("blog.example.com", RendererKind::Chrome).await;
        assert!(Arc::ptr_eq(&bare, &www));
        assert!(Arc::ptr_eq(&bare, &sub));
    }

    #[tokio::test]
    async fn registry_host_for_is_case_insensitive() {
        let reg = BreakerRegistry::with_defaults();
        let lower = reg.host_for("example.com", RendererKind::Chrome).await;
        let upper = reg.host_for("EXAMPLE.COM", RendererKind::Chrome).await;
        assert!(Arc::ptr_eq(&lower, &upper));
    }

    #[tokio::test]
    async fn registry_try_acquire_allowed_when_both_tiers_closed() {
        let reg = BreakerRegistry::with_defaults();
        assert_eq!(
            reg.try_acquire("example.com", RendererKind::Chrome).await,
            Permit::Allowed
        );
    }

    #[tokio::test]
    async fn registry_try_acquire_rejects_when_global_open() {
        let reg = BreakerRegistry::with_defaults();
        let global = reg.global_for(RendererKind::Chrome);
        for _ in 0..reg.config().min_calls {
            global.record_outcome(BreakerOutcome::RenderError);
        }
        assert!(global.is_open());
        assert_eq!(
            reg.try_acquire("example.com", RendererKind::Chrome).await,
            Permit::Rejected
        );
    }

    #[tokio::test]
    async fn registry_try_acquire_rejects_when_host_open_even_if_global_closed() {
        let reg = BreakerRegistry::with_defaults();
        let host_b = reg.host_for("bad.com", RendererKind::Chrome).await;
        for _ in 0..reg.config().min_calls {
            host_b.record_outcome(BreakerOutcome::RenderError);
        }
        assert!(host_b.is_open());
        assert!(!reg.global_for(RendererKind::Chrome).is_open());
        assert_eq!(
            reg.try_acquire("bad.com", RendererKind::Chrome).await,
            Permit::Rejected
        );
    }

    #[tokio::test]
    async fn registry_try_acquire_host_only_ignores_open_global() {
        let reg = BreakerRegistry::with_defaults();
        let global = reg.global_for(RendererKind::Chrome);
        for _ in 0..reg.config().min_calls {
            global.record_outcome(BreakerOutcome::RenderError);
        }
        assert!(global.is_open());
        // The host tier itself is untouched and closed.
        assert_eq!(
            reg.try_acquire_host_only("fresh.com", RendererKind::Chrome)
                .await,
            Permit::Allowed
        );
    }

    #[tokio::test]
    async fn registry_try_acquire_cancels_global_probe_when_host_rejects() {
        let cfg = BreakerConfig {
            min_calls: 5,
            window_size: 10,
            max_probes: 1,
            base_cooldown: Duration::from_millis(20),
            ..BreakerConfig::default()
        };
        let reg = BreakerRegistry::new(cfg);

        // Trip the global tier and let it reach HalfOpen (one probe available).
        let global = reg.global_for(RendererKind::Chrome);
        for _ in 0..5 {
            global.record_outcome(BreakerOutcome::RenderError);
        }
        std::thread::sleep(cfg.base_cooldown + Duration::from_millis(10));

        // Trip and keep the host tier hard Open (never eligible).
        let host_b = reg.host_for("bad.com", RendererKind::Chrome).await;
        for _ in 0..5 {
            host_b.record_outcome(BreakerOutcome::RenderError);
        }
        assert!(host_b.is_open());

        // try_acquire admits a global probe, then the host tier rejects — the
        // probe slot must be returned so a later legitimate probe can use it.
        assert_eq!(
            reg.try_acquire("bad.com", RendererKind::Chrome).await,
            Permit::Rejected
        );
        assert_eq!(
            global.try_acquire(),
            Permit::Probe,
            "the cancelled slot must be free again"
        );
    }

    #[tokio::test]
    async fn registry_try_acquire_is_probe_when_either_tier_probes() {
        let cfg = BreakerConfig {
            min_calls: 5,
            window_size: 10,
            max_probes: 3,
            base_cooldown: Duration::from_millis(20),
            ..BreakerConfig::default()
        };
        let reg = BreakerRegistry::new(cfg);
        let global = reg.global_for(RendererKind::Chrome);
        for _ in 0..5 {
            global.record_outcome(BreakerOutcome::RenderError);
        }
        std::thread::sleep(cfg.base_cooldown + Duration::from_millis(10));
        // Global is HalfOpen, host tier is fresh/Closed.
        assert_eq!(
            reg.try_acquire("fresh.com", RendererKind::Chrome).await,
            Permit::Probe
        );
    }

    #[tokio::test]
    async fn registry_record_outcome_updates_both_global_and_host_tiers() {
        let reg = BreakerRegistry::with_defaults();
        for _ in 0..reg.config().min_calls {
            reg.record_outcome(
                "example.com",
                RendererKind::Chrome,
                BreakerOutcome::RenderError,
            )
            .await;
        }
        assert!(reg.global_for(RendererKind::Chrome).is_open());
        assert!(
            reg.host_for("example.com", RendererKind::Chrome)
                .await
                .is_open()
        );
    }

    #[tokio::test]
    async fn registry_record_result_true_maps_to_success() {
        let reg = BreakerRegistry::with_defaults();
        reg.record_result("example.com", RendererKind::Chrome, true)
            .await;
        let snap = reg
            .host_for("example.com", RendererKind::Chrome)
            .await
            .snapshot();
        assert_eq!(snap.window_call_count, 1);
        assert_eq!(snap.window_failure_rate, 0.0);
    }

    #[tokio::test]
    async fn registry_record_result_false_maps_to_render_error_and_counts_as_failure() {
        let reg = BreakerRegistry::with_defaults();
        reg.record_result("example.com", RendererKind::Chrome, false)
            .await;
        let snap = reg
            .host_for("example.com", RendererKind::Chrome)
            .await
            .snapshot();
        assert_eq!(snap.window_call_count, 1);
        assert_eq!(snap.window_failure_rate, 1.0);
    }

    #[tokio::test]
    async fn registry_record_scoped_outcome_global_only_leaves_host_untouched() {
        let reg = BreakerRegistry::with_defaults();
        reg.record_scoped_outcome(
            "example.com",
            RendererKind::Chrome,
            Some(BreakerOutcome::RenderError),
            None,
        )
        .await;
        assert_eq!(
            reg.global_for(RendererKind::Chrome)
                .snapshot()
                .window_call_count,
            1
        );
        assert_eq!(
            reg.host_for("example.com", RendererKind::Chrome)
                .await
                .snapshot()
                .window_call_count,
            0
        );
    }

    /// A tier handed `None` must have its probe RELEASED, not left held.
    ///
    /// `try_acquire` increments `admitted` on HalfOpen and only `record_outcome`
    /// or `cancel_probe` moves it back. The scoped call sites acquire a permit
    /// for both tiers, then record to one and disarm the guard, so before this
    /// was fixed the global slot stayed held: `admitted` saturated at
    /// `max_probes`, every host got `Permit::Rejected`, and only the 30s
    /// `eval_timeout` broke it. That is a recall path, because the ladder skips
    /// a Rejected tier for every host while it lasts.
    #[tokio::test]
    async fn scoped_outcome_releases_the_global_probe_it_does_not_record() {
        let cfg = BreakerConfig {
            min_calls: 5,
            window_size: 10,
            max_probes: 1,
            base_cooldown: Duration::from_millis(20),
            ..BreakerConfig::default()
        };
        let reg = BreakerRegistry::new(cfg);
        for _ in 0..5 {
            reg.record_outcome(
                "example.com",
                RendererKind::Chrome,
                BreakerOutcome::ConnectionError,
            )
            .await;
        }
        std::thread::sleep(cfg.base_cooldown + Duration::from_millis(10));

        let (permit, guard) = reg
            .acquire_with_guard("example.com", RendererKind::Chrome)
            .await;
        assert_eq!(permit, Permit::Probe, "the global tier must be half-open");
        assert_eq!(
            reg.global_for(RendererKind::Chrome).try_acquire(),
            Permit::Rejected,
            "the probe quota is held while the attempt is in flight"
        );

        // Exactly what a host-scoped call site does: record to the host tier,
        // hand the global tier None, then disarm.
        reg.record_scoped_outcome(
            "example.com",
            RendererKind::Chrome,
            None,
            Some(BreakerOutcome::RenderError),
        )
        .await;
        guard.disarm();

        assert_ne!(
            reg.global_for(RendererKind::Chrome).try_acquire(),
            Permit::Rejected,
            "the global probe must be released when the tier is handed None, or \
             the tier stays shut for every host until eval_timeout"
        );
    }

    /// One busy domain must not be able to disable a tier for every other host.
    ///
    /// The global window counts REQUESTS, not hosts: `min_calls: 50` with
    /// `failure_rate_threshold: 0.80`. After the correct `SiteBlocked`
    /// suppression only ~214 lightpanda outcomes reached any window in 6h of
    /// production, and github.com alone contributed 38 of them, all failures, so
    /// two domains satisfied the floor on their own and tripped the tier
    /// globally. Content verdicts are host-scoped now, which is what makes this
    /// hold.
    #[tokio::test]
    async fn single_host_thin_failures_never_open_the_global_tier() {
        let reg = BreakerRegistry::with_defaults();
        for _ in 0..200 {
            reg.record_scoped_outcome(
                "onebusyhost.com",
                RendererKind::Lightpanda,
                None,
                Some(BreakerOutcome::RenderError),
            )
            .await;
        }
        assert_eq!(
            reg.global_for(RendererKind::Lightpanda).snapshot().state,
            "closed",
            "200 content failures on ONE host must not disable the tier globally"
        );
        assert_eq!(
            reg.host_for("onebusyhost.com", RendererKind::Lightpanda)
                .await
                .snapshot()
                .state,
            "open",
            "the host tier must still learn that this host is unsuitable"
        );
    }

    /// The guard that keeps the above from being a regression.
    ///
    /// A renderer that is genuinely down (dead CDP pool, lightpanda process
    /// gone, sidecar unreachable) fails on EVERY host at once and never returns
    /// an inspectable response, so its outcomes are `ConnectionError` rather
    /// than a content verdict and they must still reach the global tier. This is
    /// the 2026-06-21 class of incident and it must keep paging.
    #[tokio::test]
    async fn broken_renderer_still_opens_the_global_tier() {
        let reg = BreakerRegistry::with_defaults();
        for i in 0..50 {
            reg.record_outcome(
                &format!("host{i}.example"),
                RendererKind::Chrome,
                BreakerOutcome::ConnectionError,
            )
            .await;
        }
        assert_eq!(
            reg.global_for(RendererKind::Chrome).snapshot().state,
            "open",
            "a renderer failing across 50 distinct hosts must still trip globally"
        );
    }

    /// A tier timeout on a FULL budget is a tier signal, not a host verdict: a
    /// hung CDP pool times out exactly like this. It is what the origin-failure
    /// carve-out deliberately excludes, so it must keep advancing the global
    /// window.
    #[tokio::test]
    async fn tier_timeout_with_full_budget_still_advances_the_global_window() {
        let ctx = AttemptContext::capture(Duration::from_secs(10), Duration::from_millis(2_500));
        assert!(
            !ctx.was_clamped_by_deadline,
            "a 2.5s tier budget inside 10s of remaining is not clamped"
        );
        let outcome = classify_outcome(false, false, true, false, &ctx);
        assert_eq!(outcome, BreakerOutcome::TierTimeout);
        assert!(
            outcome.advances_window(false),
            "TierTimeout must advance the window, or a hung tier is never caught"
        );
    }

    #[tokio::test]
    async fn registry_record_scoped_outcome_host_only_leaves_global_untouched() {
        let reg = BreakerRegistry::with_defaults();
        reg.record_scoped_outcome(
            "example.com",
            RendererKind::Chrome,
            None,
            Some(BreakerOutcome::RenderError),
        )
        .await;
        assert_eq!(
            reg.global_for(RendererKind::Chrome)
                .snapshot()
                .window_call_count,
            0
        );
        assert_eq!(
            reg.host_for("example.com", RendererKind::Chrome)
                .await
                .snapshot()
                .window_call_count,
            1
        );
    }

    #[tokio::test]
    async fn registry_record_scoped_outcome_none_none_touches_nothing() {
        let reg = BreakerRegistry::with_defaults();
        reg.record_scoped_outcome("example.com", RendererKind::Chrome, None, None)
            .await;
        assert_eq!(
            reg.global_for(RendererKind::Chrome)
                .snapshot()
                .window_call_count,
            0
        );
        assert_eq!(
            reg.host_for("example.com", RendererKind::Chrome)
                .await
                .snapshot()
                .window_call_count,
            0
        );
    }

    #[tokio::test]
    async fn registry_cancel_probe_releases_both_tiers() {
        let cfg = BreakerConfig {
            min_calls: 5,
            window_size: 10,
            max_probes: 1,
            base_cooldown: Duration::from_millis(20),
            ..BreakerConfig::default()
        };
        let reg = BreakerRegistry::new(cfg);
        for _ in 0..5 {
            reg.record_outcome(
                "example.com",
                RendererKind::Chrome,
                BreakerOutcome::RenderError,
            )
            .await;
        }
        std::thread::sleep(cfg.base_cooldown + Duration::from_millis(10));
        assert_eq!(
            reg.try_acquire("example.com", RendererKind::Chrome).await,
            Permit::Probe
        );
        assert_eq!(
            reg.try_acquire("example.com", RendererKind::Chrome).await,
            Permit::Rejected
        );
        reg.cancel_probe("example.com", RendererKind::Chrome).await;
        assert_eq!(
            reg.try_acquire("example.com", RendererKind::Chrome).await,
            Permit::Probe
        );
    }

    #[tokio::test]
    async fn registry_reset_all_closes_every_open_breaker() {
        let reg = BreakerRegistry::with_defaults();
        for _ in 0..reg.config().min_calls {
            reg.record_outcome(
                "example.com",
                RendererKind::Chrome,
                BreakerOutcome::RenderError,
            )
            .await;
        }
        assert!(reg.global_for(RendererKind::Chrome).is_open());
        reg.reset_all();
        assert!(!reg.global_for(RendererKind::Chrome).is_open());
        assert!(
            !reg.host_for("example.com", RendererKind::Chrome)
                .await
                .is_open()
        );
    }

    #[tokio::test]
    async fn registry_reset_all_returns_evicted_host_count() {
        let reg = BreakerRegistry::with_defaults();
        let _ = reg.host_for("a.com", RendererKind::Chrome).await;
        let _ = reg.host_for("b.com", RendererKind::Http).await;
        reg.host.run_pending_tasks().await;
        let evicted = reg.reset_all();
        assert_eq!(evicted, 2);
    }

    #[tokio::test]
    async fn registry_snapshot_includes_global_entries_for_every_kind() {
        let reg = BreakerRegistry::with_defaults();
        let snap = reg.snapshot();
        assert_eq!(snap.global.len(), 6);
    }

    #[tokio::test]
    async fn registry_snapshot_includes_recorded_host_entries() {
        let reg = BreakerRegistry::with_defaults();
        reg.record_result("z.com", RendererKind::Chrome, true).await;
        reg.record_result("a.com", RendererKind::Http, true).await;
        reg.host.run_pending_tasks().await;
        let snap = reg.snapshot();
        assert_eq!(snap.per_host.len(), 2);
    }

    #[tokio::test]
    async fn registry_snapshot_per_host_is_sorted_by_host_then_renderer() {
        let reg = BreakerRegistry::with_defaults();
        reg.record_result("z.com", RendererKind::Chrome, true).await;
        reg.record_result("a.com", RendererKind::Http, true).await;
        reg.record_result("a.com", RendererKind::Chrome, true).await;
        reg.host.run_pending_tasks().await;
        let snap = reg.snapshot();
        let hosts: Vec<(&str, &str)> = snap
            .per_host
            .iter()
            .map(|h| (h.host.as_str(), h.renderer.as_str()))
            .collect();
        let mut sorted = hosts.clone();
        sorted.sort();
        assert_eq!(hosts, sorted);
    }

    // ── ProbeGuard ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn acquire_with_guard_disarmed_on_allowed_permit() {
        let reg = BreakerRegistry::with_defaults();
        let (permit, _guard) = reg
            .acquire_with_guard("example.com", RendererKind::Chrome)
            .await;
        assert_eq!(permit, Permit::Allowed);
        // Guard's Drop must not cancel anything meaningful since it is unarmed
        // — verified indirectly: a fresh breaker after guard drop is still
        // Allowed (an errant cancel_probe would be harmless anyway, but this
        // documents the intended contract).
    }

    #[tokio::test]
    async fn acquire_with_guard_armed_on_probe_permit_and_disarm_prevents_release() {
        let cfg = BreakerConfig {
            min_calls: 5,
            window_size: 10,
            max_probes: 1,
            base_cooldown: Duration::from_millis(20),
            ..BreakerConfig::default()
        };
        let reg = BreakerRegistry::new(cfg);
        for _ in 0..5 {
            reg.record_outcome(
                "example.com",
                RendererKind::Chrome,
                BreakerOutcome::RenderError,
            )
            .await;
        }
        std::thread::sleep(cfg.base_cooldown + Duration::from_millis(10));

        let (permit, guard) = reg
            .acquire_with_guard("example.com", RendererKind::Chrome)
            .await;
        assert_eq!(permit, Permit::Probe);
        // Quota is exhausted while the guard is alive.
        assert_eq!(
            reg.try_acquire("example.com", RendererKind::Chrome).await,
            Permit::Rejected
        );
        guard.disarm();
        // disarm() must prevent the Drop-time cancel: quota stays exhausted.
        assert_eq!(
            reg.try_acquire("example.com", RendererKind::Chrome).await,
            Permit::Rejected
        );
    }

    #[tokio::test]
    async fn probe_guard_drop_without_disarm_frees_the_probe_slot() {
        let cfg = BreakerConfig {
            min_calls: 5,
            window_size: 10,
            max_probes: 1,
            base_cooldown: Duration::from_millis(20),
            ..BreakerConfig::default()
        };
        let reg = BreakerRegistry::new(cfg);
        for _ in 0..5 {
            reg.record_outcome(
                "example.com",
                RendererKind::Chrome,
                BreakerOutcome::RenderError,
            )
            .await;
        }
        std::thread::sleep(cfg.base_cooldown + Duration::from_millis(10));

        let (permit, guard) = reg
            .acquire_with_guard("example.com", RendererKind::Chrome)
            .await;
        assert_eq!(permit, Permit::Probe);
        drop(guard); // armed, un-disarmed → must cancel on drop
        assert_eq!(
            reg.try_acquire("example.com", RendererKind::Chrome).await,
            Permit::Probe,
            "dropping an un-disarmed guard must return the probe slot"
        );
    }
}
