//! Persistent CDP WebSocket connection with single-reader event loop.
//!
//! Design: exactly one task owns the `WsRead` half. `send_recv` never reads the
//! socket — it publishes a pending `oneshot::Sender` into a shared map, writes
//! the request through the (mutex-guarded) `WsWrite`, and awaits the response
//! on the receiver. Events that arrive without a matching id are broadcast on a
//! `tokio::sync::broadcast` channel for `wait_for_event` subscribers.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use crw_core::error::{CrwError, CrwResult};
use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::{Mutex, broadcast, oneshot};
use tokio::task::JoinHandle;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const WS_CLOSE_TIMEOUT: Duration = Duration::from_secs(3);
const EVENT_CHANNEL_CAPACITY: usize = 1024;

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
        let (ws, _) = tokio::time::timeout(connect_timeout, connect_async(ws_url))
            .await
            .map_err(|_| CrwError::Timeout(connect_timeout.as_millis() as u64))?
            .map_err(|e| CrwError::RendererError(format!("CDP connect: {e}")))?;
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
