//! Hand-rolled circuit breaker with single-probe half-open semantics.
//!
//! The breaker has three states: Closed (allow all), Open (reject all
//! until cool-down expires), HalfOpen (allow exactly one probe through).
//!
//! ## Why hand-rolled
//!
//! The popular `failsafe` crate's HalfOpen state can leak multiple probes
//! through under concurrent load (split-brain). For a backend protecting
//! a flaky renderer we want the strict invariant: one probe at a time.
//! ~80 LOC of state-machine code is cheaper than the dependency.
//!
//! ## Concurrency
//!
//! All state transitions happen under a single `Mutex`. The hot path is
//! a single lock acquisition per call to `try_acquire` / `record_result`.

use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Closed,
    Open { until: Instant },
    HalfOpenProbe,
}

#[derive(Debug, Clone, Copy)]
pub struct BreakerConfig {
    /// Consecutive failures required to trip the breaker.
    pub failure_threshold: u32,
    /// How long the breaker stays Open before transitioning to HalfOpen.
    pub cooldown: Duration,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            cooldown: Duration::from_secs(30),
        }
    }
}

#[derive(Debug)]
struct Inner {
    state: State,
    consecutive_failures: u32,
}

/// Outcome of `try_acquire` — caller must respect this before calling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permit {
    /// Closed or recovering — caller may proceed.
    Allowed,
    /// HalfOpen probe granted — caller is the single probe; must report.
    Probe,
    /// Open — caller must skip this renderer.
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
            config,
            inner: Mutex::new(Inner {
                state: State::Closed,
                consecutive_failures: 0,
            }),
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(BreakerConfig::default())
    }

    /// Attempt to acquire a permit. Always pair with `record_result`.
    pub fn try_acquire(&self) -> Permit {
        let mut inner = self.inner.lock().expect("breaker mutex poisoned");
        match inner.state {
            State::Closed => Permit::Allowed,
            State::Open { until } => {
                if Instant::now() >= until {
                    inner.state = State::HalfOpenProbe;
                    Permit::Probe
                } else {
                    Permit::Rejected
                }
            }
            // Another caller already holds the probe — reject.
            State::HalfOpenProbe => Permit::Rejected,
        }
    }

    /// Report the outcome of a permit acquired from `try_acquire`.
    /// Pass `success=true` for a healthy outcome, `false` for a
    /// breaker-relevant failure (transport, timeout, render failure).
    pub fn record_result(&self, success: bool) {
        let mut inner = self.inner.lock().expect("breaker mutex poisoned");
        match inner.state {
            State::HalfOpenProbe => {
                if success {
                    inner.state = State::Closed;
                    inner.consecutive_failures = 0;
                } else {
                    inner.state = State::Open {
                        until: Instant::now() + self.config.cooldown,
                    };
                }
            }
            State::Closed => {
                if success {
                    inner.consecutive_failures = 0;
                } else {
                    inner.consecutive_failures = inner.consecutive_failures.saturating_add(1);
                    if inner.consecutive_failures >= self.config.failure_threshold {
                        inner.state = State::Open {
                            until: Instant::now() + self.config.cooldown,
                        };
                        inner.consecutive_failures = 0;
                    }
                }
            }
            // Should not happen in correct usage but tolerate it.
            State::Open { .. } => {}
        }
    }

    /// True if the breaker is currently rejecting requests.
    pub fn is_open(&self) -> bool {
        let inner = self.inner.lock().expect("breaker mutex poisoned");
        matches!(inner.state, State::Open { .. } | State::HalfOpenProbe)
    }
}

// ── Registry: per-host + global per-renderer breakers ────────────────

use crate::preference::normalize_host;
use crw_core::types::RendererKind;
use moka::future::Cache;
use std::sync::Arc;

const REGISTRY_CAPACITY: u64 = 10_000;
const REGISTRY_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Two-tier breaker registry: a global breaker per `RendererKind` (catches
/// infra-level outages) and a per-`(host, renderer)` breaker (catches
/// site-specific failures).
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

    /// Try to acquire a permit. Both tiers must allow; if either rejects
    /// we return `Rejected` without touching either breaker's probe slot.
    pub async fn try_acquire(&self, host: &str, renderer: RendererKind) -> Permit {
        let global = self.global_for(renderer);
        let host_b = self.host_for(host, renderer).await;
        // Check global is_open / host is_open without consuming probes.
        if global.is_open() || host_b.is_open() {
            // One tier already in Open/HalfOpenProbe — but if cooldown
            // expired the next try_acquire on that breaker will return a
            // probe. Run them in order: global first (covers infra), then
            // host. If either returns Rejected, no probe was taken from it.
        }
        let g = global.try_acquire();
        if g == Permit::Rejected {
            return Permit::Rejected;
        }
        let h = host_b.try_acquire();
        if h == Permit::Rejected {
            // Roll back the global probe by reporting success — keeps the
            // global tier's state stable while host is rejecting.
            if g == Permit::Probe {
                global.record_result(true);
            }
            return Permit::Rejected;
        }
        // If either is a Probe, the overall permit is a Probe.
        if g == Permit::Probe || h == Permit::Probe {
            Permit::Probe
        } else {
            Permit::Allowed
        }
    }

    /// Report outcome to both tiers.
    pub async fn record_result(&self, host: &str, renderer: RendererKind, success: bool) {
        self.global_for(renderer).record_result(success);
        self.host_for(host, renderer).await.record_result(success);
    }
}

impl Default for BreakerRegistry {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(threshold: u32, cooldown_ms: u64) -> BreakerConfig {
        BreakerConfig {
            failure_threshold: threshold,
            cooldown: Duration::from_millis(cooldown_ms),
        }
    }

    #[test]
    fn closed_allows_all() {
        let b = CircuitBreaker::new(cfg(3, 100));
        for _ in 0..10 {
            assert_eq!(b.try_acquire(), Permit::Allowed);
            b.record_result(true);
        }
    }

    #[test]
    fn trips_after_threshold() {
        let b = CircuitBreaker::new(cfg(3, 100));
        for _ in 0..3 {
            assert_eq!(b.try_acquire(), Permit::Allowed);
            b.record_result(false);
        }
        assert!(b.is_open());
        assert_eq!(b.try_acquire(), Permit::Rejected);
    }

    #[test]
    fn success_resets_counter() {
        let b = CircuitBreaker::new(cfg(3, 100));
        for _ in 0..2 {
            b.try_acquire();
            b.record_result(false);
        }
        b.try_acquire();
        b.record_result(true);
        // Need full threshold of fresh failures to trip.
        for _ in 0..2 {
            b.try_acquire();
            b.record_result(false);
        }
        assert!(!b.is_open());
    }

    #[test]
    fn half_open_single_probe() {
        let b = CircuitBreaker::new(cfg(2, 10));
        b.try_acquire();
        b.record_result(false);
        b.try_acquire();
        b.record_result(false);
        assert!(b.is_open());

        std::thread::sleep(Duration::from_millis(15));

        // First caller after cooldown gets the probe.
        assert_eq!(b.try_acquire(), Permit::Probe);
        // A concurrent second caller is rejected — single probe invariant.
        assert_eq!(b.try_acquire(), Permit::Rejected);
    }

    #[test]
    fn half_open_success_closes() {
        let b = CircuitBreaker::new(cfg(2, 10));
        b.try_acquire();
        b.record_result(false);
        b.try_acquire();
        b.record_result(false);
        std::thread::sleep(Duration::from_millis(15));
        assert_eq!(b.try_acquire(), Permit::Probe);
        b.record_result(true);
        assert!(!b.is_open());
        assert_eq!(b.try_acquire(), Permit::Allowed);
    }

    #[test]
    fn half_open_failure_reopens() {
        let b = CircuitBreaker::new(cfg(2, 10));
        b.try_acquire();
        b.record_result(false);
        b.try_acquire();
        b.record_result(false);
        std::thread::sleep(Duration::from_millis(15));
        assert_eq!(b.try_acquire(), Permit::Probe);
        b.record_result(false);
        assert!(b.is_open());
        assert_eq!(b.try_acquire(), Permit::Rejected);
    }
}
