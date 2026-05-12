//! Browser context pool — long-lived CDP connections + ephemeral browser
//! contexts amortizing per-request handshake (~200–500 ms) to ~0 ms on warm
//! hits. See `plans/tamam-browser-ppolu-detaylica-virtual-glade.md` for the
//! full design rationale; this module implements §"Design — ownership model".
//!
//! Lock discipline (load-bearing): the per-slot mutex is `std::sync::Mutex`
//! and is **never held across an `.await`**. Any change that violates that
//! invariant breaks both shutdown's force-close path and the sync `Drop`
//! emergency reaper. Audited at PR review time.
//!
//! Stealth JS injection and `Fetch.enable` are **strictly per-target session-
//! scoped**: do NOT hoist them to the connection or the browser-context level.
//! See `cdp.rs` for the per-target attach + setup sequence.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, Weak};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};

use crw_core::error::{CrwError, CrwResult};

use crate::cdp_conn::CdpConnection;

/// Operations the pool performs against a Chrome connection. Trait-extracted
/// so unit tests can substitute a fake without bringing up real CDP. The blanket
/// impl below wires it to `CdpConnection`.
#[async_trait]
pub trait ChromeConnOps: Send + Sync + 'static {
    async fn create_browser_context(&self) -> CrwResult<String>;
    async fn dispose_browser_context(&self, ctx_id: &str) -> CrwResult<()>;
    async fn close_target(&self, target_id: &str) -> CrwResult<()>;
    async fn health_check(&self) -> CrwResult<()>;
    async fn close_conn(&self);
    fn is_closed(&self) -> bool;
}

#[async_trait]
impl ChromeConnOps for CdpConnection {
    async fn create_browser_context(&self) -> CrwResult<String> {
        let v = self
            .send_recv(
                "Target.createBrowserContext",
                json!({}),
                None,
                Duration::from_secs(2),
            )
            .await?;
        v.get("browserContextId")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                CrwError::RendererError(
                    "Target.createBrowserContext: missing browserContextId".into(),
                )
            })
    }

    async fn dispose_browser_context(&self, ctx_id: &str) -> CrwResult<()> {
        self.send_recv(
            "Target.disposeBrowserContext",
            json!({ "browserContextId": ctx_id }),
            None,
            Duration::from_secs(1),
        )
        .await
        .map(|_| ())
    }

    async fn close_target(&self, target_id: &str) -> CrwResult<()> {
        self.send_recv(
            "Target.closeTarget",
            json!({ "targetId": target_id }),
            None,
            Duration::from_secs(2),
        )
        .await
        .map(|_| ())
    }

    async fn health_check(&self) -> CrwResult<()> {
        CdpConnection::health_check_browser(self, Duration::from_millis(200)).await
    }

    async fn close_conn(&self) {
        CdpConnection::close(self).await;
    }

    fn is_closed(&self) -> bool {
        CdpConnection::is_closed(self)
    }
}

/// Per-phase recycle progress marker. Drives the `pool_recycle_seconds{phase}`
/// histogram and lets shutdown reason about where a Recycling slot is parked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecyclePhase {
    CloseTarget,
    DisposeCtx,
    CreateCtx,
}

/// Slot state machine. Transitions are serialized under the per-slot
/// `StdMutex<PooledSlot>` and held only synchronously.
pub enum SlotState<C: ChromeConnOps> {
    Idle {
        conn: Arc<C>,
        ctx_id: String,
    },
    /// `target_id` starts as `None` on every checkout. `record_target` (called
    /// synchronously inside `fetch_inner` immediately after `Target.createTarget`
    /// returns Ok, before any subsequent `.await`) flips it to `Some`. There
    /// are three legitimate ways `target_id` can still be `None` at release
    /// time: (a) `fetch_inner` failed at `createTarget` so the recorder never
    /// fired; (b) `record_target` lost a theoretical shutdown race and
    /// log-and-returned per its panic-free contract; (c) emergency `Drop` ran
    /// without ever entering `fetch_inner`. In all three cases `release()`
    /// skips `closeTarget` and still disposes+recreates ctx.
    CheckedOut {
        conn: Arc<C>,
        ctx_id: String,
        target_id: Option<String>,
    },
    Recycling {
        conn: Arc<C>,
        ctx_id: String,
        target_id: Option<String>,
        phase: RecyclePhase,
    },
    /// Conn ownership is explicit: `Some(_)` means whoever transitioned to
    /// `Dead` is responsible for closing it (release runs inline; Drop spawns
    /// a reaper; shutdown takes it via `take_conn()`). After close, conn
    /// becomes `None`.
    Dead {
        conn: Option<Arc<C>>,
    },
    /// Shutdown owns the close: phase 2 (clean idle drain) or phase 3 (force).
    Closing,
}

pub struct PooledSlot<C: ChromeConnOps> {
    pub state: SlotState<C>,
    /// Single-shot terminator: only ONE of {release, drop, shutdown-force-close}
    /// may consume this. `compare_exchange(false→true, AcqRel)` decides the
    /// winner; the loser does nothing (no decrement, no notify, no close).
    /// Reset to `false` on every `Idle → CheckedOut` transition under the slot
    /// mutex (the only legitimate `true → false` transition pool-wide).
    pub terminator: AtomicBool,
    pub last_used: Instant,
}

impl<C: ChromeConnOps> PooledSlot<C> {
    /// Single API for transferring conn ownership across paths. Always called
    /// under the slot mutex (caller holds the `MutexGuard` and `DerefMut`s to
    /// `&mut PooledSlot`). Idempotent: a second call always returns `None`.
    ///
    /// State matrix:
    ///
    /// | Current state                  | Return        | New state                    |
    /// |--------------------------------|---------------|------------------------------|
    /// | `Idle { conn, .. }`            | `Some(conn)`  | `Closing`                    |
    /// | `CheckedOut { conn, .. }`      | `Some(conn)`  | `Closing`                    |
    /// | `Recycling { conn, .. }`       | `Some(conn)`  | `Closing`                    |
    /// | `Dead { conn: Some(c) }`       | `Some(c)`     | `Dead { conn: None }`        |
    /// | `Dead { conn: None }`          | `None`        | `Dead { conn: None }`        |
    /// | `Closing`                      | `None`        | `Closing`                    |
    pub fn take_conn(&mut self) -> Option<Arc<C>> {
        let cur = std::mem::replace(&mut self.state, SlotState::Closing);
        match cur {
            SlotState::Idle { conn, .. }
            | SlotState::CheckedOut { conn, .. }
            | SlotState::Recycling { conn, .. } => {
                // state already replaced with Closing
                Some(conn)
            }
            SlotState::Dead { conn: Some(c) } => {
                self.state = SlotState::Dead { conn: None };
                Some(c)
            }
            SlotState::Dead { conn: None } => {
                self.state = SlotState::Dead { conn: None };
                None
            }
            SlotState::Closing => {
                self.state = SlotState::Closing;
                None
            }
        }
    }
}

pub type Slot<C> = Arc<StdMutex<PooledSlot<C>>>;
pub type WeakSlot<C> = Weak<StdMutex<PooledSlot<C>>>;

/// Async connection factory. Returns a fully-connected `Arc<C>`. Reuses
/// `cdp::connect_with_retry`-equivalent path so the cached-WS-URL invalidation
/// from commit `b5f7bec` is preserved (do NOT bypass).
pub type ConnFactory<C> =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = CrwResult<Arc<C>>> + Send>> + Send + Sync>;

#[derive(Clone, Debug)]
pub struct PoolCfg {
    pub size: usize,
    pub recycle_after_navs: u32,
    pub idle_timeout: Duration,
    pub health_check_after: Duration,
    pub shutdown_drain: Duration,
    pub close_target_timeout: Duration,
    pub dispose_ctx_timeout: Duration,
    pub create_ctx_timeout: Duration,
}

impl Default for PoolCfg {
    fn default() -> Self {
        Self {
            size: 4,
            recycle_after_navs: 1,
            idle_timeout: Duration::from_secs(300),
            health_check_after: Duration::from_secs(60),
            shutdown_drain: Duration::from_secs(30),
            close_target_timeout: Duration::from_secs(2),
            dispose_ctx_timeout: Duration::from_secs(1),
            create_ctx_timeout: Duration::from_secs(1),
        }
    }
}

pub struct BrowserContextPool<C: ChromeConnOps> {
    sem: Arc<Semaphore>,
    idle: Arc<StdMutex<VecDeque<Slot<C>>>>,
    all_slots: Arc<StdMutex<Vec<WeakSlot<C>>>>,
    inflight: Arc<AtomicUsize>,
    conn_factory: ConnFactory<C>,
    cfg: PoolCfg,
    closed: Arc<AtomicBool>,
    notify_idle: Arc<Notify>,
}

impl<C: ChromeConnOps> BrowserContextPool<C> {
    pub fn new(cfg: PoolCfg, conn_factory: ConnFactory<C>) -> Arc<Self> {
        let sem = Arc::new(Semaphore::new(cfg.size));
        Arc::new(Self {
            sem,
            idle: Arc::new(StdMutex::new(VecDeque::new())),
            all_slots: Arc::new(StdMutex::new(Vec::new())),
            inflight: Arc::new(AtomicUsize::new(0)),
            conn_factory,
            cfg,
            closed: Arc::new(AtomicBool::new(false)),
            notify_idle: Arc::new(Notify::new()),
        })
    }

    pub fn cfg(&self) -> &PoolCfg {
        &self.cfg
    }

    pub fn inflight(&self) -> usize {
        self.inflight.load(Ordering::SeqCst)
    }

    pub fn idle_len(&self) -> usize {
        self.idle.lock().unwrap().len()
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// The single decrement+notify path. Called from `BookkeepingToken`
    /// (success and cancel paths) and directly from shutdown phase 3 when
    /// shutdown wins the terminator. Atomic ops + Notify only — never panics.
    fn dec_inflight_and_notify(&self) {
        // Saturating subtract: defensive against double-dec (which the
        // terminator AtomicBool already prevents). If a future bug ever
        // double-decs we want a stable 0 floor, not a wraparound to usize::MAX
        // that would make the shutdown drain loop spin forever.
        let prev = self.inflight.load(Ordering::SeqCst);
        if prev > 0 {
            self.inflight.fetch_sub(1, Ordering::SeqCst);
        }
        self.notify_idle.notify_waiters();
        self.refresh_gauges();
    }

    /// Snapshot inflight + idle into Prometheus gauges. Cheap (two atomic
    /// loads + lock + len + two `set` calls). Call from every mutation site
    /// so `chrome_pool_inflight` / `chrome_pool_idle` track actual state.
    fn refresh_gauges(&self) {
        let inflight = self.inflight.load(Ordering::SeqCst) as i64;
        let idle = self.idle.lock().unwrap().len() as i64;
        crw_core::metrics::metrics()
            .chrome_pool_inflight
            .set(inflight);
        crw_core::metrics::metrics().chrome_pool_idle.set(idle);
    }

    /// Walk `all_slots` and drop entries whose `Weak` is dead. Called on every
    /// new slot creation in `acquire()` (cold path) to bound the registry.
    fn prune_all_slots(&self) {
        let mut g = self.all_slots.lock().unwrap();
        g.retain(|w| w.strong_count() > 0);
    }

    /// Acquire a checked-out slot. Permit is acquired BEFORE any idle/create
    /// decision so two concurrent acquirers cannot both observe capacity and
    /// double-create. Permit is the source of truth for "you may exist in
    /// CheckedOut state".
    pub async fn acquire(self: &Arc<Self>) -> CrwResult<PoolGuard<C>> {
        let permit = self
            .sem
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| CrwError::Shutdown)?;

        // Two health-check retries max (plan §acquire flow step 3)
        for attempt in 0..3 {
            // Try popping an idle slot first
            let popped = self.idle.lock().unwrap().pop_front();
            let slot = if let Some(s) = popped {
                // Health-check if stale
                let needs_check = {
                    let g = s.lock().unwrap();
                    Instant::now().duration_since(g.last_used) > self.cfg.health_check_after
                };
                if needs_check {
                    // Snapshot conn for the async health check (no lock across await)
                    let conn = match &s.lock().unwrap().state {
                        SlotState::Idle { conn, .. } => conn.clone(),
                        _ => {
                            // Slot was somehow not Idle — defensive; treat as dead
                            self.mark_slot_dead_and_drop(&s);
                            continue;
                        }
                    };
                    if conn.health_check().await.is_err() {
                        self.mark_slot_dead_and_drop(&s);
                        if attempt < 2 {
                            continue;
                        }
                        return Err(CrwError::RendererError(
                            "pool: health check failed after retries".into(),
                        ));
                    }
                }
                s
            } else {
                // No idle slot — create a fresh one under the held permit
                let conn = (self.conn_factory)().await?;
                let ctx_id = conn.create_browser_context().await?;
                let slot = Arc::new(StdMutex::new(PooledSlot {
                    state: SlotState::Idle { conn, ctx_id },
                    terminator: AtomicBool::new(false),
                    last_used: Instant::now(),
                }));
                self.prune_all_slots();
                self.all_slots.lock().unwrap().push(Arc::downgrade(&slot));
                slot
            };

            // Transition Idle → CheckedOut under the slot mutex.
            // No `.await` between these atomic+sync operations.
            let (conn_clone, ctx_id_clone) = {
                let mut g = slot.lock().unwrap();
                // Verify state is Idle (sanity — only Idle slots are in the free-list)
                let (conn, ctx_id) = match std::mem::replace(&mut g.state, SlotState::Closing) {
                    SlotState::Idle { conn, ctx_id } => (conn, ctx_id),
                    other => {
                        // Race with shutdown phase 3 between pop and lock —
                        // restore state and retry
                        g.state = other;
                        if attempt < 2 {
                            drop(g);
                            continue;
                        }
                        return Err(CrwError::Shutdown);
                    }
                };
                // Reset terminator (only legitimate true→false transition)
                g.terminator.store(false, Ordering::SeqCst);
                let conn_clone = conn.clone();
                let ctx_id_clone = ctx_id.clone();
                g.state = SlotState::CheckedOut {
                    conn,
                    ctx_id,
                    target_id: None,
                };
                g.last_used = Instant::now();
                (conn_clone, ctx_id_clone)
            };

            self.inflight.fetch_add(1, Ordering::SeqCst);
            self.refresh_gauges();
            return Ok(PoolGuard {
                slot: Some(slot),
                permit: Some(permit),
                conn: conn_clone,
                ctx_id: ctx_id_clone,
                pool: Arc::downgrade(self),
            });
        }
        Err(CrwError::Internal("pool: acquire exhausted retries".into()))
    }

    /// Sync helper: transition slot to Dead{Some(conn)} if it currently holds a
    /// conn. Used on health-check failure (caller still holds the permit).
    fn mark_slot_dead_and_drop(&self, slot: &Slot<C>) {
        let conn_opt = {
            let mut g = slot.lock().unwrap();
            g.take_conn()
        };
        if let Some(conn) = conn_opt {
            // Best-effort close; do not block acquire path on it
            let conn_clone = conn.clone();
            if tokio::runtime::Handle::try_current().is_ok() {
                tokio::spawn(async move {
                    let _ =
                        tokio::time::timeout(Duration::from_secs(1), conn_clone.close_conn()).await;
                });
            }
        }
    }

    /// Graceful shutdown. Three phases: (1) signal close + wait for inflight
    /// to drain via Notify, (2) dispose+close every Idle slot, (3) force-close
    /// anything still held in `all_slots` past the deadline AND reconcile its
    /// inflight bookkeeping. Post-condition: `inflight==0`, `idle.is_empty()`,
    /// every slot ∈ {Closing, Dead{None}}.
    pub async fn shutdown(self: Arc<Self>, drain: Duration) {
        self.closed.store(true, Ordering::SeqCst);
        self.sem.close();
        let deadline = Instant::now() + drain;

        // Phase 1: graceful — wait for inflight to drain via Notify
        while self.inflight.load(Ordering::SeqCst) > 0 {
            if Instant::now() >= deadline {
                break;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let _ = tokio::time::timeout(remaining, self.notify_idle.notified()).await;
        }

        // Phase 2: drain whatever's idle now (released cleanly during phase 1)
        let drained: Vec<Slot<C>> = {
            let mut g = self.idle.lock().unwrap();
            std::mem::take(&mut *g).into()
        };
        for slot in drained {
            // For Idle slots, we own the conn directly; dispose ctx then close
            let (ctx_opt, conn_opt) = {
                let mut g = slot.lock().unwrap();
                let ctx = match &g.state {
                    SlotState::Idle { ctx_id, .. } => Some(ctx_id.clone()),
                    _ => None,
                };
                (ctx, g.take_conn())
            };
            if let (Some(ctx), Some(conn)) = (ctx_opt.as_ref(), conn_opt.as_ref()) {
                let _ = tokio::time::timeout(
                    self.cfg.dispose_ctx_timeout,
                    conn.dispose_browser_context(ctx),
                )
                .await;
            }
            if let Some(conn) = conn_opt {
                let _ = tokio::time::timeout(Duration::from_secs(1), conn.close_conn()).await;
            }
        }

        // Phase 3: FORCE close anything still held. Guards that never returned
        // (leaked or cancelled mid-recycle) get their conns closed AND their
        // inflight reconciled here so the pool reaches inflight==0.
        let registry: Vec<WeakSlot<C>> = {
            let mut g = self.all_slots.lock().unwrap();
            g.retain(|w| w.strong_count() > 0);
            g.clone()
        };
        for weak in registry {
            let Some(slot) = weak.upgrade() else { continue };
            let we_own_bookkeeping = slot
                .lock()
                .unwrap()
                .terminator
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok();
            let conn_opt = slot.lock().unwrap().take_conn();
            if let Some(conn) = conn_opt {
                let _ = tokio::time::timeout(Duration::from_secs(1), conn.close_conn()).await;
            }
            if we_own_bookkeeping {
                // CheckedOut/Recycling guard never returned: decrement inflight
                self.dec_inflight_and_notify();
            }
        }

        // Phase 4 (I1 fix): defensive idle-deque sweep. Phases 2 + 3 perform
        // async work between the `std::mem::take(&idle)` snapshot and the final
        // post-condition check; a release future that lost the terminator race
        // can still legally complete its happy path (state Recycling → Idle +
        // push_back) during those awaits and leave a stale Arc in the deque.
        // The slot's state is already terminal (Closing or Dead{None}) by now,
        // since phase 3 either claimed the terminator or observed release's
        // claim — so the deque entry is just a dangling Arc. Clearing it
        // satisfies the documented post-condition `idle.is_empty()` without
        // double-disposing.
        self.idle.lock().unwrap().clear();
        self.refresh_gauges();
    }
}

/// RAII bookkeeping handle. Created after release wins the terminator CAS;
/// consumed via `commit_decrement()` on normal exit. If the holding future is
/// cancelled before reaching `commit_decrement()`, `Drop` runs synchronously
/// and performs the decrement — closes the cancellation-mid-recycle gap.
///
/// Panic-free invariant: `Drop` runs during future cancellation in tokio.
/// MUST NOT panic — no unwrap, no debug_assert!. Internal helper only does
/// atomic ops + Notify.
#[must_use = "must call commit_decrement() on normal exit"]
pub(crate) struct BookkeepingToken<C: ChromeConnOps> {
    pool: Weak<BrowserContextPool<C>>,
    consumed: bool,
}

impl<C: ChromeConnOps> BookkeepingToken<C> {
    fn new(pool: Weak<BrowserContextPool<C>>) -> Self {
        Self {
            pool,
            consumed: false,
        }
    }

    fn commit_decrement(mut self) {
        if let Some(p) = self.pool.upgrade() {
            p.dec_inflight_and_notify();
        }
        self.consumed = true;
    }
}

impl<C: ChromeConnOps> Drop for BookkeepingToken<C> {
    fn drop(&mut self) {
        if !self.consumed
            && let Some(p) = self.pool.upgrade()
        {
            p.dec_inflight_and_notify();
            // pool already dropped → process is exiting, inflight irrelevant
        }
    }
}

pub struct PoolGuard<C: ChromeConnOps> {
    slot: Option<Slot<C>>,
    permit: Option<OwnedSemaphorePermit>,
    pub conn: Arc<C>,
    pub ctx_id: String,
    pool: Weak<BrowserContextPool<C>>,
}

impl<C: ChromeConnOps> PoolGuard<C> {
    /// Called synchronously inside `fetch_inner` immediately after
    /// `Target.createTarget` returns Ok, before any subsequent `.await`.
    /// Writes `Some(target_id)` into the slot's `CheckedOut` payload under the
    /// slot mutex.
    ///
    /// Panic-free contract: if the slot is no longer in `CheckedOut` state by
    /// the time this method acquires the slot mutex (theoretical shutdown
    /// race), log at WARN and return without mutating. Never panic.
    pub fn record_target(&self, target_id: String) {
        let Some(slot) = self.slot.as_ref() else {
            tracing::warn!("PoolGuard::record_target called without slot — bug");
            return;
        };
        let mut g = match slot.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(), // poison-tolerant: still try to record
        };
        if let SlotState::CheckedOut {
            target_id: ref mut tid,
            ..
        } = g.state
        {
            *tid = Some(target_id);
        } else {
            tracing::warn!("record_target: slot no longer CheckedOut (shutdown race) — skipping");
        }
    }

    /// Normal release path. Inline (no `tokio::spawn`). Sub-steps follow lock
    /// discipline + terminator AtomicBool gate + RAII `BookkeepingToken`.
    pub async fn release(mut self) -> CrwResult<()> {
        let slot = self
            .slot
            .take()
            .ok_or_else(|| CrwError::Internal("PoolGuard::release: slot already taken".into()))?;
        let permit = self.permit.take(); // dropped at function exit

        // Step 1: claim terminator. If shutdown already won, skip the rest.
        let we_won = slot
            .lock()
            .unwrap()
            .terminator
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok();
        if !we_won {
            // Shutdown already claimed — drop permit and exit; bookkeeping is
            // shutdown's job now.
            drop(permit);
            return Ok(());
        }

        // Step 2: construct BookkeepingToken — from here on, cancellation is safe.
        let token = BookkeepingToken::new(self.pool.clone());

        // Step 3: lock slot, verify CheckedOut, snapshot, transition → Recycling.
        let (conn, ctx_id, target_id_opt) = {
            let mut g = slot.lock().unwrap();
            match std::mem::replace(&mut g.state, SlotState::Closing) {
                SlotState::CheckedOut {
                    conn,
                    ctx_id,
                    target_id,
                } => {
                    let conn_c = conn.clone();
                    let ctx_c = ctx_id.clone();
                    let tid = target_id.clone();
                    g.state = SlotState::Recycling {
                        conn,
                        ctx_id,
                        target_id,
                        phase: RecyclePhase::CloseTarget,
                    };
                    (conn_c, ctx_c, tid)
                }
                other => {
                    g.state = other;
                    // shutdown-raced abort path
                    record_recycle_outcome("shutdown_raced");
                    token.commit_decrement();
                    drop(permit);
                    return Ok(());
                }
            }
        };

        let pool = match self.pool.upgrade() {
            Some(p) => p,
            None => {
                // Pool dropped — process is exiting. Best-effort close on conn.
                let _ = tokio::time::timeout(Duration::from_secs(1), conn.close_conn()).await;
                token.commit_decrement();
                drop(permit);
                return Ok(());
            }
        };
        let metrics = crw_core::metrics::metrics();

        // Step 4: closeTarget (skip if target_id is None)
        if let Some(tid) = target_id_opt.as_deref() {
            let timer = metrics
                .chrome_pool_recycle_seconds
                .with_label_values(&["close_target"])
                .start_timer();
            let res =
                tokio::time::timeout(pool.cfg.close_target_timeout, conn.close_target(tid)).await;
            timer.observe_duration();
            match res {
                Ok(Ok(())) => {}
                Ok(Err(_)) | Err(_) => {
                    // closeTarget failed or timed out — treat conn as suspect.
                    metrics
                        .chrome_pool_recycle_failures_total
                        .with_label_values(&["close_target_timeout"])
                        .inc();
                    return Self::transition_to_dead_and_close(
                        &slot,
                        &conn,
                        "dead_conn",
                        token,
                        permit,
                    )
                    .await;
                }
            }
        }

        // Reconcile-after-close
        if !Self::reconcile_set_phase(&slot, RecyclePhase::DisposeCtx) {
            return Self::handle_shutdown_raced(token, permit);
        }

        // Step 5a: disposeBrowserContext
        let timer = metrics
            .chrome_pool_recycle_seconds
            .with_label_values(&["dispose_ctx"])
            .start_timer();
        let res = tokio::time::timeout(
            pool.cfg.dispose_ctx_timeout,
            conn.dispose_browser_context(&ctx_id),
        )
        .await;
        timer.observe_duration();
        if matches!(res, Ok(Err(_)) | Err(_)) {
            metrics
                .chrome_pool_recycle_failures_total
                .with_label_values(&["dispose_ctx_fail"])
                .inc();
            return Self::transition_to_dead_and_close(&slot, &conn, "dead_conn", token, permit)
                .await;
        }

        if !Self::reconcile_set_phase(&slot, RecyclePhase::CreateCtx) {
            return Self::handle_shutdown_raced(token, permit);
        }

        // Step 5b: createBrowserContext (fresh ctx_id)
        let timer = metrics
            .chrome_pool_recycle_seconds
            .with_label_values(&["create_ctx"])
            .start_timer();
        let create_res =
            tokio::time::timeout(pool.cfg.create_ctx_timeout, conn.create_browser_context()).await;
        timer.observe_duration();
        let fresh_ctx = match create_res {
            Ok(Ok(id)) => id,
            Ok(Err(_)) | Err(_) => {
                metrics
                    .chrome_pool_recycle_failures_total
                    .with_label_values(&["create_ctx_fail"])
                    .inc();
                return Self::transition_to_dead_and_close(
                    &slot,
                    &conn,
                    "dead_conn",
                    token,
                    permit,
                )
                .await;
            }
        };

        // Final reconcile: relock; if state is still Recycling, transition →
        // Idle and push to free-list.
        let did_push = {
            let mut g = slot.lock().unwrap();
            match std::mem::replace(&mut g.state, SlotState::Closing) {
                SlotState::Recycling { conn, .. } => {
                    g.state = SlotState::Idle {
                        conn,
                        ctx_id: fresh_ctx,
                    };
                    g.last_used = Instant::now();
                    true
                }
                other => {
                    g.state = other;
                    false
                }
            }
        };

        if did_push {
            pool.idle.lock().unwrap().push_back(slot);
            pool.notify_idle.notify_waiters();
            record_recycle_outcome("success");
        } else {
            record_recycle_outcome("shutdown_raced");
        }
        pool.refresh_gauges();

        token.commit_decrement();
        drop(permit);
        Ok(())
    }

    /// Relock the slot; if state is still Recycling, advance to `next_phase`
    /// and return `true`. Otherwise return `false` (shutdown raced).
    fn reconcile_set_phase(slot: &Slot<C>, next_phase: RecyclePhase) -> bool {
        let mut g = slot.lock().unwrap();
        if let SlotState::Recycling { ref mut phase, .. } = g.state {
            *phase = next_phase;
            true
        } else {
            false
        }
    }

    /// Failure-branch close handoff: transition Recycling → Dead{Some(conn)},
    /// then take_conn back out and close on the moved value. If shutdown's
    /// take_conn beat us, our take returns None and we skip close entirely
    /// (no double-close).
    async fn transition_to_dead_and_close(
        slot: &Slot<C>,
        _conn_ref: &Arc<C>,
        outcome: &'static str,
        token: BookkeepingToken<C>,
        permit: Option<OwnedSemaphorePermit>,
    ) -> CrwResult<()> {
        // Transition Recycling → Dead{Some(conn)} (or observe shutdown raced)
        let we_have_conn = {
            let mut g = slot.lock().unwrap();
            match std::mem::replace(&mut g.state, SlotState::Closing) {
                SlotState::Recycling { conn, .. } => {
                    g.state = SlotState::Dead { conn: Some(conn) };
                    true
                }
                other => {
                    g.state = other;
                    false
                }
            }
        };
        if !we_have_conn {
            // Shutdown took over mid-failure-branch
            record_recycle_outcome("shutdown_raced");
            token.commit_decrement();
            drop(permit);
            return Ok(());
        }

        // Now take_conn back out and close
        let conn_to_close = slot.lock().unwrap().take_conn();
        if let Some(c) = conn_to_close {
            let _ = tokio::time::timeout(Duration::from_secs(1), c.close_conn()).await;
        }
        record_recycle_outcome(outcome);
        token.commit_decrement();
        drop(permit);
        Ok(())
    }

    fn handle_shutdown_raced(
        token: BookkeepingToken<C>,
        permit: Option<OwnedSemaphorePermit>,
    ) -> CrwResult<()> {
        record_recycle_outcome("shutdown_raced");
        token.commit_decrement();
        drop(permit);
        Ok(())
    }
}

fn record_recycle_outcome(outcome: &'static str) {
    crw_core::metrics::metrics()
        .chrome_pool_recycle_total
        .with_label_values(&[outcome])
        .inc();
}

impl<C: ChromeConnOps> Drop for PoolGuard<C> {
    fn drop(&mut self) {
        let Some(slot) = self.slot.take() else {
            // release() already took the slot — nothing to do here.
            return;
        };

        // Step 1: claim terminator
        let we_won = slot
            .lock()
            .unwrap()
            .terminator
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok();
        if !we_won {
            // shutdown or release already won; permit drops below
            return;
        }

        // Step 2: lock slot, take conn out, transition → Dead{Some(conn)}
        let conn_opt = {
            let mut g = slot.lock().unwrap();
            match std::mem::replace(&mut g.state, SlotState::Closing) {
                SlotState::CheckedOut { conn, .. } | SlotState::Recycling { conn, .. } => {
                    g.state = SlotState::Dead { conn: Some(conn) };
                    g.take_conn() // immediate take to Dead{None}
                }
                other => {
                    g.state = other;
                    None
                }
            }
        };

        // Step 3: metrics
        record_recycle_outcome("emergency_drop");
        crw_core::metrics::metrics()
            .chrome_pool_recycle_failures_total
            .with_label_values(&["missed_release"])
            .inc();

        // Step 4: dec inflight
        if let Some(pool) = self.pool.upgrade() {
            pool.dec_inflight_and_notify();
        }

        // Step 5: best-effort reaper for conn close
        if let Some(conn) = conn_opt
            && tokio::runtime::Handle::try_current().is_ok()
        {
            tokio::spawn(async move {
                let _ = tokio::time::timeout(Duration::from_secs(1), conn.close_conn()).await;
            });
        }
        // Step 6: permit drops with self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    /// Minimal fake connection — counts each CDP op for assertions.
    #[derive(Default)]
    struct FakeConn {
        create_ctx_calls: AtomicU32,
        dispose_ctx_calls: AtomicU32,
        close_target_calls: AtomicU32,
        close_conn_calls: AtomicU32,
        close_target_should_fail: AtomicBool,
        closed: AtomicBool,
    }

    #[async_trait]
    impl ChromeConnOps for FakeConn {
        async fn create_browser_context(&self) -> CrwResult<String> {
            let n = self.create_ctx_calls.fetch_add(1, Ordering::SeqCst);
            Ok(format!("ctx-{n}"))
        }
        async fn dispose_browser_context(&self, _ctx_id: &str) -> CrwResult<()> {
            self.dispose_ctx_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn close_target(&self, _target_id: &str) -> CrwResult<()> {
            self.close_target_calls.fetch_add(1, Ordering::SeqCst);
            if self.close_target_should_fail.load(Ordering::SeqCst) {
                Err(CrwError::RendererError("close_target forced fail".into()))
            } else {
                Ok(())
            }
        }
        async fn health_check(&self) -> CrwResult<()> {
            Ok(())
        }
        async fn close_conn(&self) {
            self.close_conn_calls.fetch_add(1, Ordering::SeqCst);
            self.closed.store(true, Ordering::SeqCst);
        }
        fn is_closed(&self) -> bool {
            self.closed.load(Ordering::SeqCst)
        }
    }

    fn fake_factory() -> ConnFactory<FakeConn> {
        Arc::new(|| Box::pin(async { Ok(Arc::new(FakeConn::default())) }))
    }

    fn small_pool_cfg() -> PoolCfg {
        PoolCfg {
            size: 2,
            recycle_after_navs: 1,
            idle_timeout: Duration::from_secs(300),
            health_check_after: Duration::from_secs(60),
            shutdown_drain: Duration::from_secs(2),
            close_target_timeout: Duration::from_millis(500),
            dispose_ctx_timeout: Duration::from_millis(500),
            create_ctx_timeout: Duration::from_millis(500),
        }
    }

    #[tokio::test]
    async fn take_conn_state_matrix() {
        let conn: Arc<FakeConn> = Arc::new(FakeConn::default());
        let mut slot = PooledSlot {
            state: SlotState::Idle {
                conn: conn.clone(),
                ctx_id: "x".into(),
            },
            terminator: AtomicBool::new(false),
            last_used: Instant::now(),
        };
        // Idle → returns Some, transitions to Closing
        assert!(slot.take_conn().is_some());
        assert!(matches!(slot.state, SlotState::Closing));
        // Closing → returns None, stays Closing
        assert!(slot.take_conn().is_none());
        assert!(matches!(slot.state, SlotState::Closing));

        // Dead{Some} → returns Some, transitions to Dead{None}
        slot.state = SlotState::Dead {
            conn: Some(conn.clone()),
        };
        assert!(slot.take_conn().is_some());
        assert!(matches!(slot.state, SlotState::Dead { conn: None }));
        // Dead{None} → returns None, stays Dead{None}
        assert!(slot.take_conn().is_none());
    }

    #[tokio::test]
    async fn acquire_release_happy_path() {
        let pool = BrowserContextPool::new(small_pool_cfg(), fake_factory());
        assert_eq!(pool.inflight(), 0);
        assert_eq!(pool.idle_len(), 0);

        let guard = pool.acquire().await.expect("acquire");
        assert_eq!(pool.inflight(), 1);
        assert_eq!(pool.idle_len(), 0);

        // Pretend fetch_inner created a target
        guard.record_target("t-1".into());
        let conn = guard.conn.clone();
        guard.release().await.expect("release");

        assert_eq!(pool.inflight(), 0);
        assert_eq!(pool.idle_len(), 1);
        assert_eq!(conn.close_target_calls.load(Ordering::SeqCst), 1);
        assert_eq!(conn.dispose_ctx_calls.load(Ordering::SeqCst), 1);
        // create_ctx: once on creation + once on recycle
        assert_eq!(conn.create_ctx_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn release_skips_close_target_when_none() {
        let pool = BrowserContextPool::new(small_pool_cfg(), fake_factory());
        let guard = pool.acquire().await.unwrap();
        let conn = guard.conn.clone();
        // Do NOT call record_target — simulates createTarget failure
        guard.release().await.unwrap();
        assert_eq!(conn.close_target_calls.load(Ordering::SeqCst), 0);
        assert_eq!(conn.dispose_ctx_calls.load(Ordering::SeqCst), 1);
        assert_eq!(pool.inflight(), 0);
        assert_eq!(pool.idle_len(), 1);
    }

    #[tokio::test]
    async fn slot_reuse_terminator_resets() {
        let pool = BrowserContextPool::new(small_pool_cfg(), fake_factory());
        let g1 = pool.acquire().await.unwrap();
        g1.record_target("t-a".into());
        g1.release().await.unwrap();
        assert_eq!(pool.inflight(), 0);

        // Second acquire should reuse the recycled slot
        let g2 = pool.acquire().await.unwrap();
        assert_eq!(pool.inflight(), 1);
        // Verify terminator was reset to false (i.e. release won the CAS again)
        g2.record_target("t-b".into());
        g2.release().await.unwrap();
        assert_eq!(pool.inflight(), 0);
        assert_eq!(pool.idle_len(), 1);
    }

    #[tokio::test]
    async fn drop_without_release_marks_dead_and_decrements() {
        let pool = BrowserContextPool::new(small_pool_cfg(), fake_factory());
        let guard = pool.acquire().await.unwrap();
        assert_eq!(pool.inflight(), 1);
        drop(guard); // emergency Drop path
        // dec_inflight runs synchronously inside Drop
        assert_eq!(pool.inflight(), 0);
        // Idle list should NOT contain the dead slot
        assert_eq!(pool.idle_len(), 0);
    }

    #[tokio::test]
    async fn close_target_failure_marks_dead_and_skips_dispose() {
        let pool = BrowserContextPool::new(small_pool_cfg(), fake_factory());
        let guard = pool.acquire().await.unwrap();
        let conn = guard.conn.clone();
        conn.close_target_should_fail.store(true, Ordering::SeqCst);
        guard.record_target("t-1".into());
        guard.release().await.unwrap();
        // close_target was called, failed
        assert_eq!(conn.close_target_calls.load(Ordering::SeqCst), 1);
        // dispose_ctx must NOT be called (suspect-conn policy)
        assert_eq!(conn.dispose_ctx_calls.load(Ordering::SeqCst), 0);
        // Conn closed
        assert_eq!(conn.close_conn_calls.load(Ordering::SeqCst), 1);
        // Slot did NOT return to idle
        assert_eq!(pool.idle_len(), 0);
        assert_eq!(pool.inflight(), 0);
    }

    #[tokio::test]
    async fn shutdown_drains_idle_slots() {
        let pool = BrowserContextPool::new(small_pool_cfg(), fake_factory());
        let g = pool.acquire().await.unwrap();
        let conn = g.conn.clone();
        g.record_target("t-1".into());
        g.release().await.unwrap();
        assert_eq!(pool.idle_len(), 1);

        pool.clone().shutdown(Duration::from_secs(1)).await;
        assert!(pool.is_closed());
        // Shutdown should have closed the conn we held
        assert!(conn.close_conn_calls.load(Ordering::SeqCst) >= 1);
    }

    #[tokio::test]
    async fn shutdown_force_closes_leaked_guard() {
        let pool = BrowserContextPool::new(small_pool_cfg(), fake_factory());
        let guard = pool.acquire().await.unwrap();
        let conn = guard.conn.clone();
        std::mem::forget(guard); // simulate leak — no Drop runs
        assert_eq!(pool.inflight(), 1);

        pool.clone().shutdown(Duration::from_millis(200)).await;
        // Phase 3 should have closed the leaked conn AND decremented inflight
        assert_eq!(pool.inflight(), 0);
        assert_eq!(conn.close_conn_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn shutdown_refuses_new_acquires() {
        let pool = BrowserContextPool::new(small_pool_cfg(), fake_factory());
        pool.clone().shutdown(Duration::from_millis(100)).await;
        let r = pool.acquire().await;
        assert!(matches!(r, Err(CrwError::Shutdown)));
    }
}
