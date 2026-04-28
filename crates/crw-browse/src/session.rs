//! Browser session registry — owns `CdpConnection`s, hands out opaque short IDs.
//!
//! A session is created per MCP client connection (or on-demand via
//! `session.new`) and survives until explicitly closed or its TTL expires.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use std::collections::{HashMap, VecDeque};

use crw_core::error::{CrwError, CrwResult};
use crw_renderer::cdp_conn::CdpConnection;
use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use serde::Serialize;
use tokio::sync::{Mutex, RwLock, broadcast};
use tokio::task::JoinHandle;
use uuid::Uuid;

/// Cap on the per-session console ring buffer. Old entries are dropped
/// silently when the cap is reached so a chatty page can't grow memory
/// unbounded.
const CONSOLE_BUFFER_CAP: usize = 200;
/// Cap on the per-session network ring buffer.
const NETWORK_BUFFER_CAP: usize = 500;
/// Hard ceiling on concurrent sessions. Once reached, `SessionRegistry::insert`
/// returns an error so a runaway client can't pin unbounded CDP connections.
/// 64 covers realistic agent fan-out; anything higher likely indicates a leak.
const MAX_SESSIONS: usize = 64;

/// One captured console message. Mirrors the shape the `console` tool emits.
#[derive(Debug, Clone, Serialize)]
pub struct ConsoleEntry {
    /// `log`, `warning`, `error`, `info`, ... — the CDP `type` field.
    pub level: String,
    /// Concatenated string view of the call's arguments. We do not preserve
    /// per-arg structure — agents almost always want the rendered text.
    pub text: String,
    /// CDP `timestamp` (monotonic ms since process start). Useful for ordering.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<f64>,
}

/// One captured network event (request or response). Both flavours use the
/// same struct so the tool can return a unified, time-ordered list.
#[derive(Debug, Clone, Serialize)]
pub struct NetworkEntry {
    /// `request` for `Network.requestWillBeSent`, `response` for
    /// `Network.responseReceived`.
    pub kind: String,
    /// CDP `requestId` — pairs request/response across the two events.
    pub request_id: String,
    pub url: String,
    /// HTTP method; only populated for `request` entries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// HTTP status code; only populated for `response` entries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// Resource type — `Document`, `Script`, `XHR`, `Fetch`, ...
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_type: Option<String>,
    /// CDP `timestamp` (monotonic ms since process start).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<f64>,
}

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
    /// current URL without round-tripping to CDP. `Arc` so the event listener
    /// can update it from `Page.frameNavigated` events without holding the
    /// whole session alive.
    last_url: Arc<RwLock<Option<String>>>,
    /// `@e<N>` -> backendDOMNodeId map populated by the most recent `tree`
    /// snapshot. Refs are session-scoped and replaced wholesale on every new
    /// snapshot — they do not survive navigation. `None` means the AX node
    /// had no DOM mapping (text fragment, virtual group); resolving such a
    /// ref returns `ELEMENT_NOT_FOUND` rather than silently no-op'ing.
    /// `Arc` so the event listener can clear it on `Page.frameNavigated` for
    /// the main frame (covers click-induced and SPA navigation, which `goto`
    /// alone cannot detect).
    ref_map: Arc<Mutex<HashMap<String, Option<i64>>>>,
    /// Highest `@e<N>` index ever produced by *any* snapshot in this session.
    /// Used to differentiate "ref never existed" (`@e9999` when max was 50 →
    /// NODE_UNKNOWN; the LLM probably hallucinated) from "ref expired" (an old
    /// `@e3` after navigation cleared the map → NODE_STALE; correct recovery
    /// is re-snapshot). Persists across `clear_ref_map` because that's exactly
    /// the post-navigation case where the distinction matters.
    max_ref: AtomicUsize,
    /// Ring buffer of console messages captured from
    /// `Runtime.consoleAPICalled`. Filled by the listener task spawned in
    /// `ensure_attached`. Wrapped in `Arc` so the listener can hold a clone
    /// without keeping the whole session alive (which would create a Drop
    /// cycle: session owns the task handle, task owns the session).
    console_buffer: Arc<Mutex<VecDeque<ConsoleEntry>>>,
    /// Ring buffer of `Network.requestWillBeSent` + `Network.responseReceived`
    /// events captured by the listener task.
    network_buffer: Arc<Mutex<VecDeque<NetworkEntry>>>,
    /// Monotonically-increasing count of every Network.* event observed by the
    /// listener task. `wait` uses this for `condition: networkidle` so it
    /// detects activity even when the ring buffer is at cap and pop_front'ing
    /// keeps the visible length constant. Cloned `Arc` so the listener can
    /// bump it without borrowing the whole session.
    network_event_count: Arc<AtomicU64>,
    /// Listener task handle — aborted on session close so it can't outlive the
    /// connection. `None` until `ensure_attached` runs for the first time.
    event_listener: Mutex<Option<JoinHandle<()>>>,
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
            last_url: Arc::new(RwLock::new(None)),
            ref_map: Arc::new(Mutex::new(HashMap::new())),
            max_ref: AtomicUsize::new(0),
            console_buffer: Arc::new(Mutex::new(VecDeque::with_capacity(CONSOLE_BUFFER_CAP))),
            network_buffer: Arc::new(Mutex::new(VecDeque::with_capacity(NETWORK_BUFFER_CAP))),
            network_event_count: Arc::new(AtomicU64::new(0)),
            event_listener: Mutex::new(None),
        }
    }

    /// Total Network.* events observed by the listener over the session's
    /// lifetime. Strictly monotonic — never decremented even when the ring
    /// buffer pops front. `wait` reads this for `condition: networkidle`,
    /// comparing the counter at two points in time without touching the
    /// buffer.
    ///
    /// `Acquire` pairs with the listener's `Release` `fetch_add` so any
    /// reader that observes a higher count is also guaranteed to see the
    /// preceding buffer push. The current `wait_network_idle` caller doesn't
    /// actually read the buffer, so for *that* path the Acquire is
    /// conservative — but the synchronization is cheap and protects future
    /// callers (e.g. a "drain on idle" tool) from a TOCTOU between the
    /// counter bump and the buffer push on weakly-ordered CPUs.
    pub fn network_event_count(&self) -> u64 {
        self.network_event_count.load(Ordering::Acquire)
    }

    /// Heartbeat: bump `last_used` only (no counter increment). Cheap to call
    /// on every tool entry; keeps the TTL sweeper from killing an actively-
    /// used session. `begin_action` is the gated variant used when the caller
    /// also needs the closing-state check; `touch` is the always-safe path.
    pub async fn touch(&self) {
        *self.last_used.write().await = Instant::now();
    }

    /// Snapshot the current console buffer (oldest first). Optionally clears
    /// the buffer after reading so subsequent calls only see new entries.
    pub async fn console_drain(&self, clear: bool) -> Vec<ConsoleEntry> {
        let mut buf = self.console_buffer.lock().await;
        let out: Vec<ConsoleEntry> = buf.iter().cloned().collect();
        if clear {
            buf.clear();
        }
        out
    }

    /// Clear the console buffer without returning its contents.
    pub async fn console_clear(&self) {
        self.console_buffer.lock().await.clear();
    }

    /// Snapshot the network buffer (oldest first). Optionally clears it.
    pub async fn network_drain(&self, clear: bool) -> Vec<NetworkEntry> {
        let mut buf = self.network_buffer.lock().await;
        let out: Vec<NetworkEntry> = buf.iter().cloned().collect();
        if clear {
            buf.clear();
        }
        out
    }

    pub async fn network_clear(&self) {
        self.network_buffer.lock().await.clear();
    }

    /// Replace the session's `@e` ref map wholesale. Called by the `tree`
    /// handler after a successful AX snapshot. Also bumps `max_ref` to the
    /// highest `@e<N>` index in the new entries (monotonically — never
    /// decreases) so a later `@e9999` lookup can be classified UNKNOWN
    /// rather than STALE when N exceeds anything we've ever produced.
    pub async fn replace_ref_map(&self, entries: Vec<(String, Option<i64>)>) {
        let mut map = self.ref_map.lock().await;
        map.clear();
        let mut local_max: usize = 0;
        for (k, v) in entries {
            if let Some(n) = parse_ref_index(&k)
                && n > local_max
            {
                local_max = n;
            }
            map.insert(k, v);
        }
        // Bump monotonically — fetch_max guarantees we never lower the
        // ceiling, so a smaller subsequent snapshot doesn't reclassify
        // previously-issued high refs as NODE_UNKNOWN.
        self.max_ref.fetch_max(local_max, Ordering::SeqCst);
    }

    /// Highest `@e<N>` index ever issued in this session. Returns 0 when no
    /// snapshot has run yet — in which case every ref is UNKNOWN.
    pub fn max_ref(&self) -> usize {
        self.max_ref.load(Ordering::SeqCst)
    }

    /// Drop all `@e<N>` mappings — called after navigation so that
    /// subsequent ref-based actions (`click`, `fill`) surface `NODE_STALE`
    /// instead of acting on the previous document's backend node IDs.
    pub async fn clear_ref_map(&self) {
        self.ref_map.lock().await.clear();
    }

    /// Look up a `@e<N>` ref. Returns `Ok(Some(id))` when the ref maps to a
    /// DOM node, `Ok(None)` when the ref exists but the AX node had no DOM
    /// counterpart (caller should surface `ELEMENT_NOT_FOUND`), and
    /// `Err(())` when the ref isn't in the map at all (caller should surface
    /// `NODE_STALE` and ask the LLM to re-snapshot).
    pub async fn lookup_ref(&self, ref_id: &str) -> Result<Option<i64>, ()> {
        let map = self.ref_map.lock().await;
        map.get(ref_id).copied().ok_or(())
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

        // Subscribe to the broadcast and spawn the listener BEFORE issuing
        // the `*.enable` calls below. Each `enable` ack on a real Chrome
        // (and even some lightpanda paths) can synchronously cause events
        // to fire — `Page.loadEventFired` from a fast about:blank, an
        // initial `Network.requestWillBeSent` for the navigation request.
        // If we enabled first and subscribed second, those events would
        // hit the broadcast before `subscribe()` returned a receiver and
        // be lost forever.
        //
        // The listener filters by `cdp_session_id` internally; events for
        // *other* sessions on the shared connection are skipped. Our own
        // session's events can't fire until `Page.enable` runs below, so
        // there's nothing to miss in the gap.
        let mut listener_slot = self.event_listener.lock().await;
        if listener_slot.is_none() {
            let rx = self.conn.subscribe();
            let sid = cdp_session_id.clone();
            let console_buf = self.console_buffer.clone();
            let network_buf = self.network_buffer.clone();
            let net_count = self.network_event_count.clone();
            let last_url = self.last_url.clone();
            let ref_map = self.ref_map.clone();
            let handle = tokio::spawn(async move {
                run_event_listener(
                    rx,
                    sid,
                    console_buf,
                    network_buf,
                    net_count,
                    last_url,
                    ref_map,
                )
                .await;
            });
            *listener_slot = Some(handle);
        }
        drop(listener_slot);

        // Enable the domains we rely on. Network is enabled here so the
        // listener task above can populate the network buffer for the
        // session's full lifetime, not just after the first `network` tool
        // call. Lightpanda treats unknown-method enables as no-ops, so this
        // is safe across backends.
        for method in [
            "Page.enable",
            "Runtime.enable",
            "Accessibility.enable",
            "Network.enable",
        ] {
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
        // Abort the event listener before tearing the connection down so it
        // can't observe a half-closed broadcast and log a spurious error.
        if let Some(h) = self.event_listener.lock().await.take() {
            h.abort();
        }
        self.conn.close().await;
    }
}

/// Long-running task that consumes the shared CDP broadcast for a single
/// session and pushes Console/Network events into the session's ring buffers.
/// Exits when the broadcast closes (connection dropped) or on `Lagged` it
/// keeps going — losing some events is preferable to hanging the listener.
async fn run_event_listener(
    mut rx: broadcast::Receiver<crw_renderer::cdp_conn::CdpEvent>,
    cdp_session_id: String,
    console_buf: Arc<Mutex<VecDeque<ConsoleEntry>>>,
    network_buf: Arc<Mutex<VecDeque<NetworkEntry>>>,
    network_event_count: Arc<AtomicU64>,
    last_url: Arc<RwLock<Option<String>>>,
    ref_map: Arc<Mutex<HashMap<String, Option<i64>>>>,
) {
    use broadcast::error::RecvError;
    loop {
        match rx.recv().await {
            Err(RecvError::Closed) => return,
            Err(RecvError::Lagged(n)) => {
                tracing::warn!(
                    lagged = n,
                    "session event listener lagged — some console/network events dropped"
                );
                continue;
            }
            Ok(ev) => {
                if ev.session_id.as_deref() != Some(&cdp_session_id) {
                    continue;
                }
                match ev.method.as_str() {
                    "Page.frameNavigated" => {
                        // Document-level nav covers `goto` and click-induced
                        // navigation (a5 R4 finding — `last_url` was stale
                        // after `<a href>` clicks because only `goto`
                        // bumped it). We update `last_url` unconditionally
                        // and clear the ref map so a subsequent `click @e3`
                        // from the pre-nav snapshot surfaces NODE_STALE
                        // instead of silently mapping to a different
                        // document's backendNodeId.
                        //
                        // SPA pushState/replaceState DOES NOT fire
                        // frameNavigated — it fires `navigatedWithinDocument`
                        // (handled in the next arm). We need both arms.
                        let frame = match ev.params.get("frame") {
                            Some(f) => f,
                            None => continue,
                        };
                        // `parentId` absent = main frame. Subframes navigate
                        // independently (iframes, embedded ads) and must
                        // not invalidate the top-level ref map.
                        if frame.get("parentId").is_some() {
                            continue;
                        }
                        if let Some(url) = frame.get("url").and_then(|v| v.as_str()) {
                            // Skip `about:blank` so a session that opens its
                            // initial blank target doesn't paper over the
                            // caller-supplied URL set via `set_last_url`
                            // before the first navigation event arrives.
                            if !url.is_empty() && url != "about:blank" {
                                *last_url.write().await = Some(url.to_string());
                            }
                        }
                        // Clear ref map even if URL was empty/about:blank —
                        // any frame nav invalidates DOM identity.
                        ref_map.lock().await.clear();
                    }
                    "Page.navigatedWithinDocument" => {
                        // Fired for hash changes AND history.pushState /
                        // replaceState (SPA routers like React Router,
                        // Next.js App Router, Vue Router). Document
                        // identity is preserved (no full reload), but the
                        // URL changed and most SPAs swap their visible
                        // component tree. R5 a4 verified: react.dev's
                        // sidebar click goes / → /reference/react/useState
                        // via pushState, and without this arm the previous
                        // `tree` snapshot's refs would still resolve to the
                        // old (mounted) DOM nodes — silently misleading the
                        // caller. Spec params: `{frameId, url}`.
                        if let Some(frame_id) = ev.params.get("frameId").and_then(|v| v.as_str()) {
                            // Best-effort main-frame check: in flat-mode
                            // sessions, the session's `frameId` equals the
                            // top frame. We don't track that here, so we
                            // accept any frame's pushState for the session
                            // — subframe SPAs are extremely rare and the
                            // false positive (clearing top-level refs on a
                            // subframe pushState) is recoverable via a
                            // single `tree` re-snapshot, whereas a missed
                            // top-frame SPA nav silently corrupts state.
                            let _ = frame_id;
                        }
                        if let Some(url) = ev.params.get("url").and_then(|v| v.as_str())
                            && !url.is_empty()
                            && url != "about:blank"
                        {
                            *last_url.write().await = Some(url.to_string());
                        }
                        ref_map.lock().await.clear();
                    }
                    "Runtime.executionContextDestroyed" => {
                        // Defense-in-depth for document recreation paths that
                        // skip both frameNavigated and navigatedWithinDocument:
                        // `document.open()`, `<iframe srcdoc>` swap, sandbox
                        // host re-mount. The dominant frame's JS context is
                        // torn down, so prior @e refs (which hold backendNodeIds
                        // anchored to that document) become unresolvable. We
                        // clear unconditionally; the false-positive cost on
                        // iframe context destruction is one extra `tree` call,
                        // and the alternative (silently mapping a stale ref to
                        // a different document's node) is far worse.
                        ref_map.lock().await.clear();
                    }
                    "Runtime.consoleAPICalled" => {
                        let entry = parse_console_event(&ev.params);
                        let mut buf = console_buf.lock().await;
                        if buf.len() >= CONSOLE_BUFFER_CAP {
                            buf.pop_front();
                        }
                        buf.push_back(entry);
                    }
                    "Network.requestWillBeSent" => {
                        if let Some(entry) = parse_network_request(&ev.params) {
                            let mut buf = network_buf.lock().await;
                            if buf.len() >= NETWORK_BUFFER_CAP {
                                buf.pop_front();
                            }
                            buf.push_back(entry);
                            // Bump the lifetime counter AFTER pushing so a
                            // wait poller observing the higher count is
                            // guaranteed to see the buffer push too. `Release`
                            // pairs with `Acquire` in
                            // `BrowserSession::network_event_count`. The
                            // counter is bumped regardless of whether the
                            // ring buffer evicted old entries — `wait`'s
                            // networkidle path needs to detect activity even
                            // when buf.len() is pinned at the cap.
                            network_event_count.fetch_add(1, Ordering::Release);
                        }
                    }
                    "Network.responseReceived" => {
                        if let Some(entry) = parse_network_response(&ev.params) {
                            let mut buf = network_buf.lock().await;
                            if buf.len() >= NETWORK_BUFFER_CAP {
                                buf.pop_front();
                            }
                            buf.push_back(entry);
                            network_event_count.fetch_add(1, Ordering::Release);
                        }
                    }
                    "Network.loadingFailed" => {
                        // Failed loads still represent network churn — count
                        // them so `wait` for `networkidle` doesn't decide a
                        // page is quiet just because every request errored.
                        // We don't push a buffer entry: there's no useful
                        // url/status to report, and adding a third
                        // `NetworkEntry::kind` would expand the public
                        // surface for one debugging signal.
                        network_event_count.fetch_add(1, Ordering::Release);
                    }
                    _ => {}
                }
            }
        }
    }
}

fn parse_console_event(params: &serde_json::Value) -> ConsoleEntry {
    let level = params
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("log")
        .to_string();
    let timestamp = params.get("timestamp").and_then(|v| v.as_f64());
    let text = params
        .get("args")
        .and_then(|v| v.as_array())
        .map(|args| {
            args.iter()
                .map(|a| {
                    a.get("value")
                        .and_then(|v| v.as_str().map(String::from).or_else(|| Some(v.to_string())))
                        .or_else(|| {
                            a.get("description")
                                .and_then(|v| v.as_str())
                                .map(String::from)
                        })
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    ConsoleEntry {
        level,
        text,
        timestamp,
    }
}

fn parse_network_request(params: &serde_json::Value) -> Option<NetworkEntry> {
    let request_id = params
        .get("requestId")
        .and_then(|v| v.as_str())?
        .to_string();
    let request = params.get("request")?;
    let url = request.get("url").and_then(|v| v.as_str())?.to_string();
    let method = request
        .get("method")
        .and_then(|v| v.as_str())
        .map(String::from);
    let resource_type = params
        .get("type")
        .and_then(|v| v.as_str())
        .map(String::from);
    let timestamp = params.get("timestamp").and_then(|v| v.as_f64());
    Some(NetworkEntry {
        kind: "request".into(),
        request_id,
        url,
        method,
        status: None,
        resource_type,
        timestamp,
    })
}

fn parse_network_response(params: &serde_json::Value) -> Option<NetworkEntry> {
    let request_id = params
        .get("requestId")
        .and_then(|v| v.as_str())?
        .to_string();
    let response = params.get("response")?;
    let url = response.get("url").and_then(|v| v.as_str())?.to_string();
    let status = response
        .get("status")
        .and_then(|v| v.as_f64())
        .and_then(|s| {
            // CDP shouldn't return out-of-range status, but defence-in-depth:
            // an `s as u16` truncation on garbage values would yield bogus
            // small status codes (e.g. -1 → 65535, 700 → 700, 70000 → 4464).
            if (0.0..=65_535.0).contains(&s) {
                Some(s as u16)
            } else {
                None
            }
        });
    let resource_type = params
        .get("type")
        .and_then(|v| v.as_str())
        .map(String::from);
    let timestamp = params.get("timestamp").and_then(|v| v.as_f64());
    Some(NetworkEntry {
        kind: "response".into(),
        request_id,
        url,
        method: None,
        status,
        resource_type,
        timestamp,
    })
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
        // Cap the session count. Without this, an LLM that loops on
        // `session.new` (or a buggy embedder) can pin unbounded CDP
        // connections, each holding open WebSocket + Chromium target +
        // ring buffers. 64 is well above realistic agent fan-out; if a real
        // workload hits this ceiling, that's the right time to raise it
        // explicitly rather than silently grow.
        //
        // The pre-check is a fast path; the post-insert verify below shrinks
        // the TOCTOU window where N concurrent callers all observe `len ==
        // MAX-1` on the pre-check and then all proceed to insert. It does
        // not fully close it — DashMap's `len()` sums shard-local counts
        // without a global lock, so two threads inserting into different
        // shards near the boundary can both see `len <= MAX_SESSIONS` and
        // briefly leave the map at `MAX_SESSIONS + 1` between the two
        // observations. We accept that at-most-one-over slack: the cap is a
        // soft guard against runaway loops, not a hard isolation boundary,
        // and tightening it would require a global Mutex on every insert.
        if self.primary.len() >= MAX_SESSIONS {
            return Err(CrwError::RendererError(format!(
                "session limit reached ({MAX_SESSIONS}) — close stale sessions first"
            )));
        }
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
                    // Post-insert verify: under contention, the pre-check
                    // above is racy. If the cap was crossed by concurrent
                    // inserts, undo our claim and report the limit.
                    if self.primary.len() > MAX_SESSIONS {
                        self.primary.remove(&id);
                        self.by_short.remove(&session.short_id);
                        return Err(CrwError::RendererError(format!(
                            "session limit reached ({MAX_SESSIONS}) — close stale sessions first"
                        )));
                    }
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

/// Parse the integer N out of an `@e<N>` ref string. Returns `None` for any
/// other shape (`@c1`, `e5`, garbage). Centralised so the session and
/// resolve_ref agree on what counts as a ref index.
pub fn parse_ref_index(ref_id: &str) -> Option<usize> {
    let stripped = ref_id.strip_prefix("@e")?;
    stripped.parse::<usize>().ok()
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

    use crw_renderer::cdp_conn::CdpEvent;
    use tokio::sync::broadcast;

    /// Spin up the event listener with a fresh broadcast channel and the
    /// minimum scaffolding needed to assert ref_map / last_url behavior.
    /// Returns a sender + the shared state handles. Caller drops the sender
    /// to terminate the listener.
    #[allow(clippy::type_complexity)]
    fn spawn_listener(
        sid: &str,
    ) -> (
        broadcast::Sender<CdpEvent>,
        Arc<Mutex<HashMap<String, Option<i64>>>>,
        Arc<RwLock<Option<String>>>,
        Arc<AtomicU64>,
        Arc<Mutex<VecDeque<NetworkEntry>>>,
        tokio::task::JoinHandle<()>,
    ) {
        let (tx, rx) = broadcast::channel(64);
        let ref_map: Arc<Mutex<HashMap<String, Option<i64>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let last_url: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));
        let console_buf: Arc<Mutex<VecDeque<ConsoleEntry>>> = Arc::new(Mutex::new(VecDeque::new()));
        let network_buf: Arc<Mutex<VecDeque<NetworkEntry>>> = Arc::new(Mutex::new(VecDeque::new()));
        let net_count: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
        let handle = tokio::spawn(run_event_listener(
            rx,
            sid.to_string(),
            console_buf,
            network_buf.clone(),
            net_count.clone(),
            last_url.clone(),
            ref_map.clone(),
        ));
        (tx, ref_map, last_url, net_count, network_buf, handle)
    }

    /// Wait for the listener to drain a single event. The listener is async
    /// and consumes from a broadcast channel — `yield_now` once is not
    /// guaranteed to observe the send. Polling with a tiny sleep is
    /// deterministic enough for these tests and bounded.
    ///
    /// Takes an async closure so callers can `.await` directly on
    /// `tokio::sync::Mutex` / `RwLock` without nesting executors. An earlier
    /// revision wrapped the predicate in `futures::executor::block_on` —
    /// that mixed the futures-rs mini-executor with the tokio runtime and
    /// is documented as a deadlock hazard with `tokio::sync` primitives
    /// (the futures executor doesn't drive tokio reactors).
    async fn wait_for<F, Fut>(mut check: F)
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        for _ in 0..50 {
            if check().await {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        panic!("listener never observed expected state within 100ms");
    }

    #[tokio::test]
    async fn frame_navigated_clears_ref_map_and_updates_last_url() {
        let (tx, ref_map, last_url, _nc, _nb, _h) = spawn_listener("S1");
        ref_map.lock().await.insert("e1".into(), Some(42));

        tx.send(CdpEvent {
            method: "Page.frameNavigated".into(),
            params: serde_json::json!({
                "frame": { "url": "https://example.com/", "id": "F1" }
            }),
            session_id: Some("S1".into()),
        })
        .unwrap();

        let rm = ref_map.clone();
        let lu = last_url.clone();
        wait_for(|| {
            let rm = rm.clone();
            let lu = lu.clone();
            async move {
                rm.lock().await.is_empty()
                    && lu.read().await.as_deref() == Some("https://example.com/")
            }
        })
        .await;
    }

    #[tokio::test]
    async fn subframe_nav_does_not_clear_top_level_refs() {
        let (tx, ref_map, last_url, _nc, _nb, _h) = spawn_listener("S2");
        ref_map.lock().await.insert("e1".into(), Some(7));
        ref_map.lock().await.insert("e2".into(), Some(8));

        // Send the negative case — a subframe nav that MUST be ignored.
        tx.send(CdpEvent {
            method: "Page.frameNavigated".into(),
            params: serde_json::json!({
                "frame": {
                    "url": "https://ad.example/",
                    "id": "subF",
                    "parentId": "F1"
                }
            }),
            session_id: Some("S2".into()),
        })
        .unwrap();

        // Synchronize on a SECOND event we expect to be processed: a
        // network request bumps the counter. When the counter ticks, we
        // know the listener has drained both events in order — pure
        // sleep-based polling has CI flakes; this is happens-before.
        tx.send(CdpEvent {
            method: "Network.requestWillBeSent".into(),
            params: serde_json::json!({
                "requestId": "sync",
                "request": { "url": "https://x", "method": "GET" }
            }),
            session_id: Some("S2".into()),
        })
        .unwrap();
        let nc = _nc.clone();
        wait_for(|| {
            let nc = nc.clone();
            async move { nc.load(Ordering::Acquire) >= 1 }
        })
        .await;

        assert_eq!(ref_map.lock().await.len(), 2);
        assert!(last_url.read().await.is_none());
    }

    #[tokio::test]
    async fn navigated_within_document_clears_ref_map() {
        let (tx, ref_map, last_url, _nc, _nb, _h) = spawn_listener("S3");
        ref_map.lock().await.insert("e2".into(), Some(99));

        tx.send(CdpEvent {
            method: "Page.navigatedWithinDocument".into(),
            params: serde_json::json!({
                "frameId": "F1",
                "url": "https://example.com/about"
            }),
            session_id: Some("S3".into()),
        })
        .unwrap();

        let rm = ref_map.clone();
        let lu = last_url.clone();
        wait_for(|| {
            let rm = rm.clone();
            let lu = lu.clone();
            async move {
                rm.lock().await.is_empty()
                    && lu.read().await.as_deref() == Some("https://example.com/about")
            }
        })
        .await;
    }

    #[tokio::test]
    async fn execution_context_destroyed_clears_ref_map() {
        let (tx, ref_map, _lu, _nc, _nb, _h) = spawn_listener("S4");
        ref_map.lock().await.insert("e3".into(), Some(1));
        ref_map.lock().await.insert("e4".into(), Some(2));

        tx.send(CdpEvent {
            method: "Runtime.executionContextDestroyed".into(),
            params: serde_json::json!({ "executionContextId": 5 }),
            session_id: Some("S4".into()),
        })
        .unwrap();

        let rm = ref_map.clone();
        wait_for(|| {
            let rm = rm.clone();
            async move { rm.lock().await.is_empty() }
        })
        .await;
    }

    #[tokio::test]
    async fn events_for_other_sessions_are_ignored() {
        let (tx, ref_map, _lu, net_count, _nb, _h) = spawn_listener("MINE");
        ref_map.lock().await.insert("e1".into(), Some(1));

        // Foreign session — must be skipped.
        tx.send(CdpEvent {
            method: "Page.frameNavigated".into(),
            params: serde_json::json!({ "frame": { "url": "https://x/", "id": "F" } }),
            session_id: Some("OTHER".into()),
        })
        .unwrap();
        // Sentinel for OUR session — when this lands, we know the listener
        // has already drained the foreign event ahead of it.
        tx.send(CdpEvent {
            method: "Network.requestWillBeSent".into(),
            params: serde_json::json!({
                "requestId": "sync",
                "request": { "url": "https://x", "method": "GET" }
            }),
            session_id: Some("MINE".into()),
        })
        .unwrap();
        let nc = net_count.clone();
        wait_for(|| {
            let nc = nc.clone();
            async move { nc.load(Ordering::Acquire) >= 1 }
        })
        .await;

        assert_eq!(ref_map.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn about_blank_does_not_overwrite_last_url() {
        let (tx, _rm, last_url, _nc, _nb, _h) = spawn_listener("S5");
        *last_url.write().await = Some("https://prev.example/".into());

        // about:blank — must NOT overwrite last_url.
        tx.send(CdpEvent {
            method: "Page.frameNavigated".into(),
            params: serde_json::json!({
                "frame": { "url": "about:blank", "id": "F1" }
            }),
            session_id: Some("S5".into()),
        })
        .unwrap();
        // Sentinel: a real URL arrives next. When last_url flips to it,
        // we know the about:blank event was definitely seen and skipped.
        tx.send(CdpEvent {
            method: "Page.frameNavigated".into(),
            params: serde_json::json!({
                "frame": { "url": "https://sentinel/", "id": "F2" }
            }),
            session_id: Some("S5".into()),
        })
        .unwrap();
        let lu = last_url.clone();
        wait_for(|| {
            let lu = lu.clone();
            async move { lu.read().await.as_deref() == Some("https://sentinel/") }
        })
        .await;

        // The sentinel proves about:blank was processed (and ignored)
        // before the real URL was applied — never set as last_url.
    }

    #[tokio::test]
    async fn network_request_event_increments_count_and_buffer() {
        let (tx, _rm, _lu, net_count, network_buf, _h) = spawn_listener("S6");

        tx.send(CdpEvent {
            method: "Network.requestWillBeSent".into(),
            params: serde_json::json!({
                "requestId": "req-1",
                "request": { "url": "https://api.example/data", "method": "GET" }
            }),
            session_id: Some("S6".into()),
        })
        .unwrap();

        let nc = net_count.clone();
        let nb = network_buf.clone();
        wait_for(|| {
            let nc = nc.clone();
            let nb = nb.clone();
            async move { nc.load(Ordering::Acquire) == 1 && nb.lock().await.len() == 1 }
        })
        .await;
    }
}
