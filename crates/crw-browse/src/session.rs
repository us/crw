//! Browser session registry — owns `CdpConnection`s, hands out opaque short IDs.
//!
//! A session is created per MCP client connection (or on-demand via
//! `session.new`) and survives until explicitly closed or its TTL expires.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crw_core::error::{CrwError, CrwResult};
use crw_renderer::cdp_conn::CdpConnection;
use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use uuid::Uuid;

/// Default idle TTL for sessions that have no activity.
const DEFAULT_IDLE_TTL: Duration = Duration::from_secs(600);
/// Interval at which the cleanup task sweeps for expired sessions.
const CLEANUP_INTERVAL: Duration = Duration::from_secs(30);
/// Number of retries when generating a short_id before giving up.
const SHORT_ID_RETRIES: u32 = 5;
/// base62 alphabet size used for short_id generation.
const SHORT_ID_LEN: usize = 4;

/// A single browser session — one CDP WebSocket, one ref registry.
pub struct BrowserSession {
    pub id: Uuid,
    /// 4-char base62 token shown to the LLM (stable for the session's lifetime).
    pub short_id: String,
    pub conn: Arc<CdpConnection>,
    pub is_closing: AtomicBool,
    pub created_at: Instant,
    pub last_used: RwLock<Instant>,
    action_counter: AtomicU64,
    /// CDP targetId (populated after `ensure_attached`).
    target_id: RwLock<Option<String>>,
    /// CDP flat-mode sessionId on the attached target.
    cdp_session_id: RwLock<Option<String>>,
    /// Serializes concurrent `ensure_attached` callers so only one target is
    /// ever created per session (otherwise two parallel `goto` calls both see
    /// `cdp_session_id = None` and each spawn their own target).
    attach_lock: Mutex<()>,
    /// Last URL the session navigated to — used so `tree` can report the
    /// current URL without round-tripping to CDP.
    last_url: RwLock<Option<String>>,
}

impl BrowserSession {
    pub fn new(id: Uuid, short_id: String, conn: Arc<CdpConnection>) -> Self {
        let now = Instant::now();
        Self {
            id,
            short_id,
            conn,
            is_closing: AtomicBool::new(false),
            created_at: now,
            last_used: RwLock::new(now),
            action_counter: AtomicU64::new(0),
            target_id: RwLock::new(None),
            cdp_session_id: RwLock::new(None),
            attach_lock: Mutex::new(()),
            last_url: RwLock::new(None),
        }
    }

    pub async fn last_url(&self) -> Option<String> {
        self.last_url.read().await.clone()
    }

    pub async fn set_last_url(&self, url: impl Into<String>) {
        *self.last_url.write().await = Some(url.into());
    }

    pub async fn target_id(&self) -> Option<String> {
        self.target_id.read().await.clone()
    }

    pub async fn cdp_session_id(&self) -> Option<String> {
        self.cdp_session_id.read().await.clone()
    }

    /// Lazily create a blank target and attach to it, caching both ids.
    /// No-op on subsequent calls. The `attach_lock` mutex serializes
    /// concurrent callers so we never open two targets for the same session.
    pub async fn ensure_attached(&self, timeout: Duration) -> CrwResult<String> {
        if let Some(sid) = self.cdp_session_id().await {
            return Ok(sid);
        }
        let _guard = self.attach_lock.lock().await;
        // Double-check under the mutex: another caller may have completed
        // attachment while we were waiting for the lock.
        if let Some(sid) = self.cdp_session_id().await {
            return Ok(sid);
        }

        let create = self
            .conn
            .send_recv(
                "Target.createTarget",
                serde_json::json!({ "url": "about:blank" }),
                None,
                timeout,
            )
            .await?;
        let target_id = create
            .get("targetId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CrwError::RendererError("CDP createTarget: no targetId".into()))?
            .to_string();

        let attach = self
            .conn
            .send_recv(
                "Target.attachToTarget",
                serde_json::json!({ "targetId": &target_id, "flatten": true }),
                None,
                timeout,
            )
            .await?;
        let cdp_session_id = attach
            .get("sessionId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CrwError::RendererError("CDP attach: no sessionId".into()))?
            .to_string();

        // Enable the domains we rely on for Phase 1 tools.
        for method in ["Page.enable", "Runtime.enable", "Accessibility.enable"] {
            self.conn
                .send_recv(
                    method,
                    serde_json::json!({}),
                    Some(&cdp_session_id),
                    timeout,
                )
                .await?;
        }

        *self.target_id.write().await = Some(target_id);
        *self.cdp_session_id.write().await = Some(cdp_session_id.clone());
        Ok(cdp_session_id)
    }

    /// Called before dispatching any work. Returns an error if the session is
    /// already closing so concurrent actions can't race teardown.
    pub async fn begin_action(&self) -> CrwResult<u64> {
        if self.is_closing.load(Ordering::SeqCst) {
            return Err(CrwError::RendererError(
                "session is closing or closed".into(),
            ));
        }
        *self.last_used.write().await = Instant::now();
        Ok(self.action_counter.fetch_add(1, Ordering::SeqCst))
    }

    pub async fn close(&self) {
        self.is_closing.store(true, Ordering::SeqCst);
        self.conn.close().await;
    }
}

pub struct SessionRegistry {
    primary: Arc<DashMap<Uuid, Arc<BrowserSession>>>,
    /// Secondary index: short_id -> uuid. Kept in lockstep with `primary`.
    by_short: Arc<DashMap<String, Uuid>>,
    idle_ttl: Duration,
    cleanup_task: Mutex<Option<JoinHandle<()>>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self::with_ttl(DEFAULT_IDLE_TTL)
    }

    pub fn with_ttl(idle_ttl: Duration) -> Self {
        Self {
            primary: Arc::new(DashMap::new()),
            by_short: Arc::new(DashMap::new()),
            idle_ttl,
            cleanup_task: Mutex::new(None),
        }
    }

    /// Spawn the TTL cleanup loop. Safe to call once; subsequent calls are no-op.
    pub async fn start_cleanup(self: &Arc<Self>) {
        let mut slot = self.cleanup_task.lock().await;
        if slot.is_some() {
            return;
        }
        let this = Arc::clone(self);
        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(CLEANUP_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                this.sweep_expired().await;
            }
        });
        *slot = Some(handle);
    }

    /// Close every session whose `last_used` is older than `idle_ttl`.
    ///
    /// Snapshots `(uuid, Arc<BrowserSession>)` pairs first and drops the
    /// DashMap iterator **before** taking any `.await`. Holding DashMap shard
    /// locks across `await` points (which the previous version did, via
    /// `last_used.read().await` inside `primary.iter()`) is a concurrency
    /// anti-pattern — a blocked reader can stall other shard users.
    ///
    /// Visibility: `pub` because integration tests in `tests/` sit in a
    /// separate crate. External callers should not drive this directly —
    /// the ticker spawned by `start_cleanup` is the intended entry point.
    pub async fn sweep_expired(&self) {
        let snapshot: Vec<(Uuid, Arc<BrowserSession>)> = self
            .primary
            .iter()
            .map(|e| (*e.key(), e.value().clone()))
            .collect();
        let mut victims: Vec<(Uuid, Arc<BrowserSession>)> = Vec::new();
        {
            let now = Instant::now();
            for (id, session) in snapshot {
                let last = *session.last_used.read().await;
                if now.duration_since(last) > self.idle_ttl {
                    victims.push((id, session));
                }
            }
        }
        // Re-check `last_used` under a fresh `now` just before closing: between
        // snapshot time and here, an in-flight `begin_action` may have bumped
        // the session's `last_used` forward. Without this guard an active
        // session could be closed under a racing request. The re-check keeps
        // the sweep's decision window as short as the outer lock allows.
        for (id, session) in victims {
            let last = *session.last_used.read().await;
            if Instant::now().duration_since(last) > self.idle_ttl {
                self.close_session(id).await;
            }
        }
    }

    /// Insert a new session built from `conn`. Retries on short_id collision.
    ///
    /// Ordering matters: we insert into `primary` **before** claiming the
    /// `by_short` entry. This guarantees that any reader who observes a
    /// short_id in `by_short` will also find the corresponding session in
    /// `primary` — so `get_by_short` can never return `None` for an id that
    /// `by_short` still holds. DashMap's `entry` API makes the check-then-
    /// insert on `by_short` atomic under the shard lock, so two concurrent
    /// callers cannot both claim the same short_id.
    pub fn insert(&self, conn: Arc<CdpConnection>) -> CrwResult<Arc<BrowserSession>> {
        for _ in 0..SHORT_ID_RETRIES {
            let short_id = generate_short_id();
            let id = Uuid::new_v4();
            // Reserve the short_id first under the shard lock. If Occupied,
            // another caller won the race for this id — retry with a new one.
            match self.by_short.entry(short_id.clone()) {
                Entry::Occupied(_) => continue,
                Entry::Vacant(v) => {
                    let session = Arc::new(BrowserSession::new(id, short_id, conn.clone()));
                    // Primary first — if a reader races ahead and sees the
                    // short_id after v.insert below, primary already has the
                    // session, so get_by_short is consistent.
                    self.primary.insert(id, session.clone());
                    v.insert(id);
                    return Ok(session);
                }
            }
        }
        Err(CrwError::RendererError(
            "failed to allocate unique session short_id after retries".into(),
        ))
    }

    pub fn get(&self, id: &Uuid) -> Option<Arc<BrowserSession>> {
        self.primary.get(id).map(|e| e.clone())
    }

    pub fn get_by_short(&self, short_id: &str) -> Option<Arc<BrowserSession>> {
        let id = *self.by_short.get(short_id)?;
        self.get(&id)
    }

    pub fn len(&self) -> usize {
        self.primary.len()
    }

    pub fn is_empty(&self) -> bool {
        self.primary.is_empty()
    }

    /// Remove a session and close its connection. Both indexes are cleaned.
    pub async fn close_session(&self, id: Uuid) {
        let Some((_, session)) = self.primary.remove(&id) else {
            return;
        };
        self.by_short.remove(&session.short_id);
        session.close().await;
    }
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for SessionRegistry {
    fn drop(&mut self) {
        // Best-effort: abort the cleanup task so it doesn't outlive the registry.
        if let Ok(mut slot) = self.cleanup_task.try_lock()
            && let Some(h) = slot.take()
        {
            h.abort();
        }
    }
}

fn generate_short_id() -> String {
    // 4-char base62: 62^4 ≈ 14.7M combinations. The input `u32` (~4.29B values)
    // is more than enough entropy for the 4-char prefix we actually keep; we
    // only slice the first `SHORT_ID_LEN` chars of the encoded string. Small
    // inputs encode to fewer than 4 chars, so we zero-pad on the right to
    // guarantee fixed length. Distribution isn't perfectly uniform across all
    // 14.7M slots (padded IDs cluster at the low end), but `SHORT_ID_RETRIES`
    // collision loop makes this a non-issue in practice.
    let n: u64 = rand::random::<u32>() as u64;
    let encoded = base62::encode(n);
    let mut s: String = encoded.chars().take(SHORT_ID_LEN).collect();
    while s.len() < SHORT_ID_LEN {
        s.push('0');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_id_is_fixed_length() {
        for _ in 0..1000 {
            let s = generate_short_id();
            assert_eq!(
                s.len(),
                SHORT_ID_LEN,
                "short_id must be {SHORT_ID_LEN} chars"
            );
        }
    }

    #[test]
    fn short_id_is_base62_charset() {
        for _ in 0..1000 {
            let s = generate_short_id();
            assert!(
                s.chars().all(|c| c.is_ascii_alphanumeric()),
                "non-base62 char in {s}"
            );
        }
    }

    // Registry tests that don't need a real CDP connection are deferred
    // to the integration tests in T14 (session_lifecycle.rs).
}
