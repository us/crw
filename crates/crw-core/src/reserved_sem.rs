//! A two-lane reserved semaphore: bounds total concurrency while guaranteeing
//! [`ScrapeClass::Interactive`] traffic a protected slice that
//! [`ScrapeClass::Batch`] can never hold.
//!
//! Shape (Postgres `superuser_reserved_connections` model): one `shared`
//! semaphore of `total` permits that EVERY caller acquires, plus a `batch_gate`
//! of `total - reserved` that ONLY batch acquires first. Because batch can hold
//! at most `total - reserved` `shared` permits, at least `reserved` are always
//! reachable by interactive. This is a *static capacity* guarantee, not a
//! priority queue — tokio's `Semaphore` is FIFO, so under interactive's own
//! saturation a new interactive waiter still queues; the guarantee is that
//! batch can never be the reason interactive has zero capacity.
//!
//! Asymmetric by design: interactive is protected from batch, not vice-versa.
//! `batch_gate` is floored at 1 so batch degrades but never deadlocks.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::scrape_class::ScrapeClass;

/// Record a reserved-lane acquire wait into `crw_reserved_lane_wait_seconds`.
fn record_lane_wait(lane: &str, class: ScrapeClass, waited: Duration) {
    let class = match class {
        ScrapeClass::Interactive => "interactive",
        ScrapeClass::Batch => "batch",
    };
    crate::metrics::metrics()
        .reserved_lane_wait_seconds
        .with_label_values(&[lane, class])
        .observe(waited.as_secs_f64());
}

/// A permit held by a batch task: both the gate and the shared permit, dropped
/// as a unit. Field order = drop order (Rust drops fields in declaration
/// order), so `shared` releases before `gate` — matching the stated intent.
/// (Drop order is cosmetic here: `Semaphore::drop` is synchronous with no
/// `.await` between, so no waiter observes a half-released state. The named
/// struct just keeps a future refactor from acquiring `shared` without the
/// gate.)
#[derive(Debug)]
pub struct BatchPermit {
    #[allow(dead_code)]
    shared: OwnedSemaphorePermit,
    #[allow(dead_code)]
    gate: OwnedSemaphorePermit,
}

/// An acquired permit from a [`ReservedSemaphore`], regardless of class.
#[derive(Debug)]
pub enum LanePermit {
    /// Interactive: holds one `shared` permit.
    Interactive(#[allow(dead_code)] OwnedSemaphorePermit),
    /// Batch: holds a gate permit + a shared permit.
    Batch(#[allow(dead_code)] BatchPermit),
}

/// Two-lane reserved semaphore. See module docs.
#[derive(Debug, Clone)]
pub struct ReservedSemaphore {
    shared: Arc<Semaphore>,
    batch_gate: Arc<Semaphore>,
    /// Lane name for the `crw_reserved_lane_wait_seconds{lane}` metric
    /// (e.g. "extract", "pdf", "llm", "host").
    name: &'static str,
}

impl ReservedSemaphore {
    /// Build a reserved semaphore with `total` permits, reserving `reserved`
    /// for interactive. `name` labels the wait metric.
    ///
    /// - `total` is floored at 1 (a zero-capacity pool would deadlock everyone).
    /// - `reserved` is clamped to `total - 1` so `batch_gate >= 1` always: batch
    ///   degrades under a large reserve but never deadlocks, even at
    ///   `total == 2, reserved == 1` or a misconfigured `reserved >= total`.
    /// - `reserved == 0` makes `batch_gate == total`, i.e. the two lanes
    ///   collapse to a single FIFO pool identical to today's behaviour — the
    ///   documented per-lane disable / rollback value.
    pub fn new(total: usize, reserved: usize, name: &'static str) -> Self {
        let total = total.max(1);
        let reserved = reserved.min(total - 1);
        Self {
            shared: Arc::new(Semaphore::new(total)),
            batch_gate: Arc::new(Semaphore::new(total - reserved)),
            name,
        }
    }

    /// Acquire a permit for `class`. Interactive takes only a `shared` permit;
    /// batch takes a `batch_gate` permit first (bounding batch's share of
    /// `shared`), then a `shared` permit. Held until the returned [`LanePermit`]
    /// is dropped. Records the wait into `crw_reserved_lane_wait_seconds`.
    ///
    /// Read `class` on the async side BEFORE any `spawn_blocking` — the
    /// [`crate::scrape_class::REQUEST_CLASS`] task-local does not cross that
    /// boundary.
    pub async fn acquire(&self, class: ScrapeClass) -> LanePermit {
        let t0 = std::time::Instant::now();
        let permit = match class {
            ScrapeClass::Interactive => {
                let shared = Semaphore::acquire_owned(self.shared.clone())
                    .await
                    .expect("shared reserved-lane semaphore never closed");
                LanePermit::Interactive(shared)
            }
            ScrapeClass::Batch => {
                let gate = Semaphore::acquire_owned(self.batch_gate.clone())
                    .await
                    .expect("batch_gate semaphore never closed");
                let shared = Semaphore::acquire_owned(self.shared.clone())
                    .await
                    .expect("shared reserved-lane semaphore never closed");
                LanePermit::Batch(BatchPermit { shared, gate })
            }
        };
        record_lane_wait(self.name, class, t0.elapsed());
        permit
    }

    /// Permits currently available in the shared pool (for metrics).
    pub fn available(&self) -> usize {
        self.shared.available_permits()
    }

    /// Permits currently available in the batch gate (for metrics).
    pub fn batch_available(&self) -> usize {
        self.batch_gate.available_permits()
    }
}

/// A reserved lane placed IN FRONT of a pool that already owns its own
/// concurrency semaphore (the Chrome render pools). Rather than replace the
/// pool's semaphore (which may be a checkout source-of-truth), a batch caller
/// first takes a `gate` permit sized `total - reserved`, then proceeds into the
/// existing pool acquire; an interactive caller skips the gate. Because batch
/// holds at most `total - reserved` gate permits and each gate holder occupies
/// at most one pool slot, interactive always finds ≥`reserved` pool slots free —
/// the same guarantee as [`ReservedSemaphore`] without touching the pool's
/// internals. The gate permit must be held for the whole fetch (bind it
/// alongside the pool guard).
#[derive(Debug, Clone)]
pub struct BatchGate {
    gate: Arc<Semaphore>,
    /// Lane name for the wait metric (e.g. "render").
    name: &'static str,
}

impl BatchGate {
    /// Gate sized `total - reserved`, floored at 1 so batch never deadlocks.
    /// `reserved == 0` makes the gate == `total` (no reservation). `name` labels
    /// the wait metric.
    pub fn new(total: usize, reserved: usize, name: &'static str) -> Self {
        let total = total.max(1);
        let reserved = reserved.min(total - 1);
        Self {
            gate: Arc::new(Semaphore::new((total - reserved).max(1))),
            name,
        }
    }

    /// Batch takes a gate permit (bounding batch's share of the downstream
    /// pool); interactive returns `None` and goes straight to the pool. Read
    /// `class` on the async side before any `spawn_blocking`. Records the batch
    /// wait into `crw_reserved_lane_wait_seconds`.
    pub async fn enter(&self, class: ScrapeClass) -> Option<OwnedSemaphorePermit> {
        match class {
            ScrapeClass::Interactive => None,
            ScrapeClass::Batch => {
                let t0 = std::time::Instant::now();
                let permit = Semaphore::acquire_owned(self.gate.clone())
                    .await
                    .expect("batch render gate never closed");
                record_lane_wait(self.name, class, t0.elapsed());
                Some(permit)
            }
        }
    }

    /// Gate permits currently available (for metrics).
    pub fn available(&self) -> usize {
        self.gate.available_permits()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_gate_floored_at_one() {
        // reserved >= total must never zero the batch gate (deadlock guard).
        for (total, reserved) in [(2, 1), (2, 5), (1, 0), (1, 9), (4, 4)] {
            let rs = ReservedSemaphore::new(total, reserved, "test");
            assert!(
                rs.batch_available() >= 1,
                "batch_gate must be >=1 for total={total} reserved={reserved}"
            );
        }
    }

    #[test]
    fn reserved_zero_is_single_pool() {
        // reserved=0 => both lanes see the full total (legacy FIFO behaviour).
        let rs = ReservedSemaphore::new(8, 0, "test");
        assert_eq!(rs.available(), 8);
        assert_eq!(rs.batch_available(), 8);
    }

    #[tokio::test]
    async fn interactive_reserve_survives_batch_saturation() {
        // total=4, reserve 1 => batch_gate=3. Saturate the batch lane with 3
        // held batch permits; an interactive acquire must still succeed.
        let rs = ReservedSemaphore::new(4, 1, "test");
        let mut held = Vec::new();
        for _ in 0..3 {
            held.push(rs.acquire(ScrapeClass::Batch).await);
        }
        // batch_gate now exhausted; a 4th batch acquire would block.
        assert_eq!(rs.batch_available(), 0);
        // Interactive still gets in immediately (the reserved shared permit).
        let interactive = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            rs.acquire(ScrapeClass::Interactive),
        )
        .await;
        assert!(
            interactive.is_ok(),
            "interactive must acquire while batch_gate is saturated"
        );
        drop(held);
    }

    #[tokio::test]
    async fn batch_gate_reserves_pool_slots_for_interactive() {
        // total=4, reserve 1 => gate=3. Batch can hold at most 3 gate permits,
        // so at least 1 downstream pool slot is always free for interactive.
        let g = BatchGate::new(4, 1, "test");
        let mut held = Vec::new();
        for _ in 0..3 {
            held.push(g.enter(ScrapeClass::Batch).await);
        }
        assert_eq!(g.available(), 0, "gate exhausted after 3 batch holders");
        // Interactive never touches the gate.
        assert!(g.enter(ScrapeClass::Interactive).await.is_none());
        drop(held);
    }

    #[tokio::test]
    async fn batch_gate_floored_never_zero() {
        // pool_size=1 => gate floored at 1 (batch not deadlocked).
        let g = BatchGate::new(1, 1, "test");
        assert!(g.available() >= 1);
    }

    #[tokio::test]
    async fn task_local_class_drives_lane_end_to_end() {
        // Proves the full mechanism the real lanes use: REQUEST_CLASS scope →
        // current_scrape_class() → correct lane. total=2, reserve 1 => batch_gate=1.
        let rs = ReservedSemaphore::new(2, 1, "test");
        // A batch-scoped acquire takes the batch lane and exhausts batch_gate.
        let _batch = crate::scrape_class::REQUEST_CLASS
            .scope(ScrapeClass::Batch, async {
                rs.acquire(crate::current_scrape_class()).await
            })
            .await;
        assert_eq!(rs.batch_available(), 0, "batch lane exhausted");
        // An interactive-scoped acquire takes the interactive lane and gets its
        // reserved slot immediately despite the batch lane being full.
        let got = crate::scrape_class::REQUEST_CLASS
            .scope(ScrapeClass::Interactive, async {
                tokio::time::timeout(
                    std::time::Duration::from_millis(200),
                    rs.acquire(crate::current_scrape_class()),
                )
                .await
            })
            .await;
        assert!(
            got.is_ok(),
            "interactive lane must not be blocked by a saturated batch lane"
        );
    }

    #[tokio::test]
    async fn batch_blocked_when_gate_full_then_freed() {
        // A queued batch waiter proceeds once a held batch permit is released.
        let rs = ReservedSemaphore::new(2, 1, "test"); // batch_gate = 1
        let first = rs.acquire(ScrapeClass::Batch).await;
        let rs2 = rs.clone();
        let waiter = tokio::spawn(async move { rs2.acquire(ScrapeClass::Batch).await });
        // Give the waiter a chance to block.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!waiter.is_finished(), "second batch acquire should block");
        drop(first);
        let _second = tokio::time::timeout(std::time::Duration::from_millis(200), waiter)
            .await
            .expect("waiter should finish after release")
            .expect("join ok");
    }
}
