//! Persistent CDP WebSocket connection with single-reader event loop.
//!
//! Design: exactly one task owns the `WsRead` half. `send_recv` never reads the
//! socket — it publishes a pending `oneshot::Sender` into a shared map, writes
//! the request through the (mutex-guarded) `WsWrite`, and awaits the response
//! on the receiver. Events that arrive without a matching id are broadcast on a
//! `tokio::sync::broadcast` channel for `wait_for_event` subscribers.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex as StdMutex, Weak};
use std::time::Duration;

/// Process-wide registry of live CDP connections. Each `CdpConnection` is
/// per-fetch (opened in `fetch_with_ws`, closed in the same call), so any
/// telemetry sampler needs a way to find currently-live connections without
/// holding strong references that would extend their lifetime.
///
/// The registry stores a `Weak` to the per-connection `pending` map (which is
/// the natural liveness sentinel — it's dropped when both the `CdpConnection`
/// and its event loop are gone) plus a clone of the broadcast `Sender` so we
/// can read `receiver_count()` without touching the connection.
pub(crate) struct LiveConnEntry {
    pub pending: Weak<DashMap<u64, oneshot::Sender<CdpResult>>>,
    pub events: broadcast::Sender<CdpEvent>,
}

pub(crate) static LIVE_CONNS: LazyLock<StdMutex<Vec<LiveConnEntry>>> =
    LazyLock::new(|| StdMutex::new(Vec::new()));

/// Snapshot live connections, GCing dead entries inline.
/// Returns (live_count, pending_total, subscribers_total).
pub fn snapshot_live_conns() -> (usize, usize, usize) {
    let mut g = LIVE_CONNS.lock().unwrap();
    // Drop entries whose `pending` Arc is gone — i.e. CdpConnection + its
    // event loop have both been dropped.
    g.retain(|e| e.pending.strong_count() > 0);
    let mut pending_total = 0usize;
    let mut subs_total = 0usize;
    for e in g.iter() {
        if let Some(p) = e.pending.upgrade() {
            pending_total += p.len();
        }
        subs_total += e.events.receiver_count();
    }
    (g.len(), pending_total, subs_total)
}

use crw_core::error::{CrwError, CrwResult};
use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::{Mutex, broadcast, oneshot};
use tokio::task::JoinHandle;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const WS_CLOSE_TIMEOUT: Duration = Duration::from_secs(3);
const EVENT_CHANNEL_CAPACITY: usize = 1024;

/// Classify a tungstenite error into a category name without leaking the
/// underlying URL or HTTP response details. Detail goes to traces; this
/// returns just the category for inclusion in user-facing errors.
fn classify_ws_error(e: &tokio_tungstenite::tungstenite::Error) -> &'static str {
    use tokio_tungstenite::tungstenite::Error as E;
    match e {
        E::ConnectionClosed | E::AlreadyClosed => "connection closed",
        E::Io(_) => "io error",
        E::Tls(_) => "tls error",
        E::Url(_) => "invalid websocket url",
        E::Http(_) => "http handshake rejected",
        E::HttpFormat(_) => "http format error",
        E::Capacity(_) => "message too large",
        E::Protocol(_) => "websocket protocol error",
        E::WriteBufferFull(_) => "write buffer full",
        E::Utf8(_) => "invalid utf-8",
        E::AttackAttempt => "rejected websocket attack",
    }
}

/// Rewrite a `ws://host:port/...` URL so its host is an IP literal.
///
/// Chromium 148+ guards the DevTools WebSocket against DNS-rebinding by
/// validating the `Host` header: only `localhost` and IP literals pass, so a
/// connect over a docker service name (`ws://chrome:9222/...`) is rejected with
/// an HTTP handshake error. tungstenite derives the Host header from the URL
/// authority, so resolving the hostname to an IP here makes the header an IP
/// literal Chromium accepts. Best-effort: on any parse/DNS failure (or a host
/// that is already an IP), the input is returned unchanged.
async fn resolve_ws_host_to_ip(ws_url: &str) -> String {
    let Ok(parsed) = url::Url::parse(ws_url) else {
        return ws_url.to_string();
    };
    let Some(host) = parsed.host_str().map(str::to_string) else {
        return ws_url.to_string();
    };
    // Already an IP literal (v4, or v6 which url reports without brackets).
    if host.parse::<std::net::IpAddr>().is_ok() {
        return ws_url.to_string();
    }
    let port = parsed.port().unwrap_or(9222);
    let Ok(mut addrs) = tokio::net::lookup_host((host.as_str(), port)).await else {
        return ws_url.to_string();
    };
    let Some(addr) = addrs.next() else {
        return ws_url.to_string();
    };
    let mut out = parsed;
    if out.set_ip_host(addr.ip()).is_err() {
        return ws_url.to_string();
    }
    out.to_string()
}

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
type WsWrite = futures::stream::SplitSink<WsStream, Message>;
type WsRead = futures::stream::SplitStream<WsStream>;

/// Event or response payload from the remote CDP peer.
#[derive(Debug, Clone)]
pub struct CdpEvent {
    pub method: String,
    pub params: serde_json::Value,
    pub session_id: Option<String>,
}

/// Result type delivered through a pending oneshot: Ok(result) or Err(message).
pub type CdpResult = Result<serde_json::Value, String>;

pub struct CdpConnection {
    write: Arc<Mutex<WsWrite>>,
    pending: Arc<DashMap<u64, oneshot::Sender<CdpResult>>>,
    events: broadcast::Sender<CdpEvent>,
    next_id: Arc<AtomicU64>,
    is_closed: Arc<AtomicBool>,
    event_loop: Option<JoinHandle<()>>,
}

impl CdpConnection {
    /// Open a WebSocket to the given CDP endpoint and spawn the reader loop.
    pub async fn connect(ws_url: &str, connect_timeout: Duration) -> CrwResult<Self> {
        // Chromium 148+ rejects the CDP WebSocket upgrade when the `Host` header
        // is a bare hostname (a DNS-rebinding guard: only localhost / IP literals
        // pass). The managed stack connects via a static IP so it's unaffected,
        // but a self-host compose points the engine at the `chrome` service name,
        // which then fails with "http handshake rejected". Resolve the host to an
        // IP so the handshake Host header is an IP literal Chromium accepts.
        let ws_url = resolve_ws_host_to_ip(ws_url).await;
        let (ws, _) = tokio::time::timeout(connect_timeout, connect_async(ws_url.as_str()))
            .await
            .map_err(|_| CrwError::Timeout(connect_timeout.as_millis() as u64))?
            .map_err(|e| {
                // tungstenite's Display can echo the full ws_url back (Url
                // variant) or HTTP response details. The ws_url may be
                // attacker-influenceable via config / proxy headers, so we
                // log the raw error for operators and surface only a
                // sanitized class name to the caller. This keeps prod
                // error responses free of WebSocket URLs and embedded paths.
                tracing::warn!(error = %e, "CDP connect failed");
                CrwError::RendererError(format!("CDP connect failed: {}", classify_ws_error(&e)))
            })?;
        let (write, read) = ws.split();

        let write = Arc::new(Mutex::new(write));
        let pending: Arc<DashMap<u64, oneshot::Sender<CdpResult>>> = Arc::new(DashMap::new());
        let (events_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        let is_closed = Arc::new(AtomicBool::new(false));

        let event_loop = tokio::spawn(run_event_loop(
            read,
            pending.clone(),
            events_tx.clone(),
            is_closed.clone(),
        ));

        // Register with the process-wide live-connection registry. The Weak
        // is held until the connection's pending Arc has refcount 0 (i.e.
        // both this struct and its event loop are gone), at which point
        // snapshot_live_conns() GCs the entry.
        LIVE_CONNS.lock().unwrap().push(LiveConnEntry {
            pending: Arc::downgrade(&pending),
            events: events_tx.clone(),
        });

        Ok(Self {
            write,
            pending,
            events: events_tx,
            next_id: Arc::new(AtomicU64::new(1)),
            is_closed,
            event_loop: Some(event_loop),
        })
    }

    /// Send a CDP command and await its response. Events are filtered out by
    /// the event loop — this call only completes on a message with matching id.
    pub async fn send_recv(
        &self,
        method: &str,
        params: serde_json::Value,
        session_id: Option<&str>,
        timeout: Duration,
    ) -> CrwResult<serde_json::Value> {
        if self.is_closed.load(Ordering::SeqCst) {
            return Err(CrwError::RendererError("CDP connection closed".into()));
        }

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let mut req = serde_json::json!({
            "id": id,
            "method": method,
            "params": params,
        });
        if let Some(sid) = session_id {
            req["sessionId"] = serde_json::Value::String(sid.to_string());
        }

        let (tx, rx) = oneshot::channel::<CdpResult>();
        self.pending.insert(id, tx);
        // RAII cleanup: if the caller's future is dropped (cancel) between here
        // and the `rx` await below, the guard removes the pending entry so it
        // doesn't leak. On normal response delivery, `dispatch` already
        // removed the entry — `pending.remove` is then a cheap no-op.
        let _cleanup = PendingCleanup {
            pending: &self.pending,
            id,
        };

        {
            let mut write = self.write.lock().await;
            if let Err(e) = write.send(Message::Text(req.to_string().into())).await {
                return Err(CrwError::RendererError(format!("WS send ({method}): {e}")));
            }
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(val))) => Ok(val),
            Ok(Ok(Err(msg))) => Err(CrwError::RendererError(format!("CDP {method}: {msg}"))),
            Ok(Err(_)) => Err(CrwError::RendererError(
                "CDP response channel dropped".into(),
            )),
            Err(_) => Err(CrwError::Timeout(timeout.as_millis() as u64)),
        }
    }

    /// Subscribe to the broadcast of all non-response (event) messages.
    pub fn subscribe(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }

    /// Wait for an event that satisfies `pred`, or time out.
    pub async fn wait_for_event<F>(&self, mut pred: F, timeout: Duration) -> CrwResult<CdpEvent>
    where
        F: FnMut(&CdpEvent) -> bool,
    {
        let mut rx = self.subscribe();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Err(_) => return Err(CrwError::Timeout(timeout.as_millis() as u64)),
                Ok(Err(broadcast::error::RecvError::Closed)) => {
                    return Err(CrwError::RendererError("event channel closed".into()));
                }
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Ok(ev)) => {
                    if pred(&ev) {
                        return Ok(ev);
                    }
                }
            }
        }
    }

    /// Gracefully close the WebSocket and mark the connection unusable.
    pub async fn close(&self) {
        if self.is_closed.swap(true, Ordering::SeqCst) {
            return;
        }
        let mut write = self.write.lock().await;
        let _ = tokio::time::timeout(WS_CLOSE_TIMEOUT, write.close()).await;
    }

    pub fn is_closed(&self) -> bool {
        self.is_closed.load(Ordering::SeqCst)
    }

    /// Liveness probe used by the browser-context pool when an idle slot has
    /// been parked longer than `health_check_secs`. Issues `Browser.getVersion`
    /// — a no-side-effect call that exercises both the WS write path and the
    /// reader loop. The 200 ms ceiling keeps acquire-path latency bounded.
    pub async fn health_check_browser(&self, timeout: Duration) -> CrwResult<()> {
        self.send_recv("Browser.getVersion", serde_json::json!({}), None, timeout)
            .await
            .map(|_| ())
    }
}

/// Removes a pending entry on drop. Ensures cancel-safety of `send_recv`:
/// if the caller's future is dropped while awaiting, the oneshot sender is
/// dropped from the map instead of leaking for the connection's lifetime.
struct PendingCleanup<'a> {
    pending: &'a DashMap<u64, oneshot::Sender<CdpResult>>,
    id: u64,
}

impl Drop for PendingCleanup<'_> {
    fn drop(&mut self) {
        self.pending.remove(&self.id);
    }
}

impl Drop for CdpConnection {
    fn drop(&mut self) {
        self.is_closed.store(true, Ordering::SeqCst);
        // Drain pending oneshot senders BEFORE aborting the event loop. If we
        // aborted first, every waiting `send_recv` would only learn about
        // closure when its channel was dropped — surfacing as a generic
        // "response channel dropped" rather than the real cause. Explicitly
        // delivering an error here gives callers a meaningful message even
        // though Drop can't `await` and the event loop's own drain pass will
        // never get a chance to run.
        //
        // Race with `dispatch`: the event loop may still be running and
        // calling `pending.remove(&id)` as we iterate here. That's fine —
        // `DashMap::remove` is atomic and returns an `Option`, so each `tx`
        // is consumed by exactly one path (dispatch's `Ok`, our `Err`, or
        // run_event_loop's exit drain). No double-send is possible, and
        // `let _ = tx.send(...)` is a no-op if the receiver already went away.
        let keys: Vec<u64> = self.pending.iter().map(|e| *e.key()).collect();
        for k in keys {
            if let Some((_, tx)) = self.pending.remove(&k) {
                let _ = tx.send(Err("CDP connection dropped".into()));
            }
        }
        if let Some(h) = self.event_loop.take() {
            h.abort();
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCdpMessage {
    id: Option<u64>,
    method: Option<String>,
    #[serde(default)]
    params: serde_json::Value,
    result: Option<serde_json::Value>,
    error: Option<serde_json::Value>,
    session_id: Option<String>,
}

/// Single-reader loop: routes responses back to their `oneshot::Sender`
/// (keyed by id) and broadcasts everything else as an event.
async fn run_event_loop(
    mut read: WsRead,
    pending: Arc<DashMap<u64, oneshot::Sender<CdpResult>>>,
    events: broadcast::Sender<CdpEvent>,
    is_closed: Arc<AtomicBool>,
) {
    while let Some(msg) = read.next().await {
        let text = match msg {
            Ok(Message::Text(text)) => text,
            Ok(Message::Close(_)) | Err(_) => break,
            _ => continue,
        };
        if let Ok(raw) = serde_json::from_str::<RawCdpMessage>(&text) {
            dispatch(raw, &pending, &events);
        }
    }
    is_closed.store(true, Ordering::SeqCst);
    // Drain pending: nothing else will ever complete them.
    let keys: Vec<u64> = pending.iter().map(|e| *e.key()).collect();
    for k in keys {
        if let Some((_, tx)) = pending.remove(&k) {
            let _ = tx.send(Err("WS closed".into()));
        }
    }
}

fn dispatch(
    raw: RawCdpMessage,
    pending: &DashMap<u64, oneshot::Sender<CdpResult>>,
    events: &broadcast::Sender<CdpEvent>,
) {
    if let Some(id) = raw.id {
        if let Some((_, tx)) = pending.remove(&id) {
            let res = if let Some(err) = raw.error {
                Err(err.to_string())
            } else {
                Ok(raw.result.unwrap_or(serde_json::Value::Null))
            };
            let _ = tx.send(res);
        }
    } else if let Some(method) = raw.method {
        let _ = events.send(CdpEvent {
            method,
            params: raw.params,
            session_id: raw.session_id,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;

    fn parse(json: &str) -> RawCdpMessage {
        serde_json::from_str(json).expect("valid RawCdpMessage")
    }

    #[tokio::test]
    async fn resolve_ws_host_ip_is_noop() {
        // Already an IP literal → unchanged (the managed stack's path).
        let u = "ws://172.30.40.31:9222/devtools/browser/abc";
        assert_eq!(resolve_ws_host_to_ip(u).await, u);
        // Unparseable → returned as-is, never panics.
        assert_eq!(resolve_ws_host_to_ip("not a url").await, "not a url");
    }

    #[tokio::test]
    async fn resolve_ws_host_localhost_becomes_ip() {
        // A resolvable hostname is rewritten to an IP literal so Chromium's
        // Host-header rebinding guard accepts the handshake.
        let out = resolve_ws_host_to_ip("ws://localhost:9222/devtools/browser/x").await;
        assert!(
            out.starts_with("ws://127.0.0.1:9222/") || out.starts_with("ws://[::1]:9222/"),
            "localhost should resolve to a loopback IP literal, got {out}"
        );
        assert!(out.ends_with("/devtools/browser/x"));
    }

    #[tokio::test]
    async fn dispatch_routes_response_by_id() {
        let pending: DashMap<u64, oneshot::Sender<CdpResult>> = DashMap::new();
        let (events_tx, _rx) = broadcast::channel(16);
        let (tx, rx) = oneshot::channel::<CdpResult>();
        pending.insert(7, tx);

        dispatch(
            parse(r#"{"id":7,"result":{"value":42}}"#),
            &pending,
            &events_tx,
        );

        let got = rx.await.unwrap().unwrap();
        assert_eq!(got["value"], 42);
        assert!(pending.is_empty(), "pending entry consumed on delivery");
    }

    #[tokio::test]
    async fn dispatch_forwards_error_to_pending() {
        let pending: DashMap<u64, oneshot::Sender<CdpResult>> = DashMap::new();
        let (events_tx, _rx) = broadcast::channel(16);
        let (tx, rx) = oneshot::channel::<CdpResult>();
        pending.insert(1, tx);

        dispatch(
            parse(r#"{"id":1,"error":{"code":-32000,"message":"bad"}}"#),
            &pending,
            &events_tx,
        );

        let got = rx.await.unwrap();
        assert!(got.is_err());
        assert!(got.unwrap_err().contains("bad"));
    }

    #[tokio::test]
    async fn dispatch_broadcasts_event_without_id() {
        let pending: DashMap<u64, oneshot::Sender<CdpResult>> = DashMap::new();
        let (events_tx, mut rx) = broadcast::channel(16);

        dispatch(
            parse(
                r#"{"method":"Page.loadEventFired","params":{"timestamp":1.0},"sessionId":"s1"}"#,
            ),
            &pending,
            &events_tx,
        );

        let ev = rx.recv().await.unwrap();
        assert_eq!(ev.method, "Page.loadEventFired");
        assert_eq!(ev.session_id.as_deref(), Some("s1"));
        assert_eq!(ev.params["timestamp"], 1.0);
    }

    #[tokio::test]
    async fn dispatch_drops_response_with_no_pending_entry() {
        // Late/duplicate response: must not panic, must not leak.
        let pending: DashMap<u64, oneshot::Sender<CdpResult>> = DashMap::new();
        let (events_tx, _rx) = broadcast::channel(16);
        dispatch(parse(r#"{"id":999,"result":{}}"#), &pending, &events_tx);
        assert!(pending.is_empty());
    }
}
