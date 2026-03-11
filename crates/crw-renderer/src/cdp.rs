use async_trait::async_trait;
use crw_core::error::{CrwError, CrwResult};
use crw_core::types::FetchResult;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::traits::PageFetcher;

/// Timeout for WebSocket connect handshake.
const WS_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Timeout for graceful WebSocket close.
const WS_CLOSE_TIMEOUT: Duration = Duration::from_secs(3);
/// Extra overhead budget for the overall fetch timeout (on top of page_timeout + wait_for).
const FETCH_OVERHEAD: Duration = Duration::from_secs(15);
/// Timeout for the Target.closeTarget cleanup command.
const TARGET_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
/// Default JS wait time if not specified by the caller.
const DEFAULT_JS_WAIT_MS: u64 = 2000;

/// Lightweight CDP client that talks directly to any CDP-compatible browser
/// (LightPanda, Chrome, Playwright) via WebSocket.
///
/// Uses a semaphore to limit concurrent connections to `pool_size`,
/// preventing connection storms under heavy concurrent crawl loads.
pub struct CdpRenderer {
    name: String,
    ws_url: String,
    page_timeout: Duration,
    conn_semaphore: Arc<Semaphore>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct CdpMessage {
    id: Option<u64>,
    method: Option<String>,
    params: Option<serde_json::Value>,
    result: Option<serde_json::Value>,
    error: Option<serde_json::Value>,
    session_id: Option<String>,
}

type WsWrite = futures::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;
type WsRead = futures::stream::SplitStream<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
>;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Send a CDP command and wait for the response with matching id, skipping events.
async fn cdp_send_recv(
    write: &mut WsWrite,
    read: &mut WsRead,
    method: &str,
    params: serde_json::Value,
    session_id: Option<&str>,
    timeout: Duration,
) -> CrwResult<serde_json::Value> {
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    let mut req = serde_json::json!({ "id": id, "method": method, "params": params });
    if let Some(session_id) = session_id {
        req["sessionId"] = serde_json::Value::String(session_id.to_string());
    }

    write
        .send(Message::Text(req.to_string().into()))
        .await
        .map_err(|e| CrwError::RendererError(format!("WS send ({method}): {e}")))?;

    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let msg = tokio::time::timeout_at(deadline, read.next())
            .await
            .map_err(|_| CrwError::Timeout(timeout.as_millis() as u64))?
            .ok_or_else(|| CrwError::RendererError("WS closed".into()))?
            .map_err(|e| CrwError::RendererError(format!("WS read: {e}")))?;

        if let Message::Text(text) = msg
            && let Ok(resp) = serde_json::from_str::<CdpMessage>(&text)
            && resp.id == Some(id)
        {
            if let Some(err) = resp.error {
                return Err(CrwError::RendererError(format!("CDP {method}: {err}")));
            }
            return Ok(resp.result.unwrap_or(serde_json::Value::Null));
        }
    }
}

async fn wait_for_page_ready(
    read: &mut WsRead,
    session_id: &str,
    timeout: Duration,
) -> CrwResult<u16> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut main_document_status = None;

    loop {
        let msg = tokio::time::timeout_at(deadline, read.next())
            .await
            .map_err(|_| CrwError::Timeout(timeout.as_millis() as u64))?
            .ok_or_else(|| CrwError::RendererError("WS closed".into()))?
            .map_err(|e| CrwError::RendererError(format!("WS read: {e}")))?;

        if let Message::Text(text) = msg
            && let Ok(resp) = serde_json::from_str::<CdpMessage>(&text)
        {
            if resp.session_id.as_deref() != Some(session_id) {
                continue;
            }

            match resp.method.as_deref() {
                Some("Network.responseReceived") => {
                    let params = resp.params.unwrap_or_default();
                    let is_document = params
                        .get("type")
                        .and_then(|value| value.as_str())
                        .is_some_and(|value| value == "Document");
                    if is_document {
                        main_document_status = params
                            .get("response")
                            .and_then(|response| response.get("status"))
                            .and_then(|status| status.as_f64())
                            .map(|status| status as u16)
                            .or(main_document_status);
                    }
                }
                Some("Page.loadEventFired") => {
                    return Ok(main_document_status.unwrap_or(200));
                }
                Some("Inspector.targetCrashed") => {
                    return Err(CrwError::RendererError(
                        "Target crashed during render".into(),
                    ));
                }
                _ => {}
            }
        }
    }
}

/// Close WebSocket with a timeout to prevent hanging.
async fn close_ws(write: &mut WsWrite) {
    let _ = tokio::time::timeout(WS_CLOSE_TIMEOUT, write.close()).await;
}

async fn close_target(write: &mut WsWrite, read: &mut WsRead, target_id: &str) {
    let _ = cdp_send_recv(
        write,
        read,
        "Target.closeTarget",
        serde_json::json!({ "targetId": target_id }),
        None,
        TARGET_CLOSE_TIMEOUT,
    )
    .await;
}

impl CdpRenderer {
    pub fn new(name: &str, ws_url: &str, page_timeout_ms: u64, pool_size: usize) -> Self {
        let pool_size = pool_size.max(1);
        Self {
            name: name.to_string(),
            ws_url: ws_url.to_string(),
            page_timeout: Duration::from_millis(page_timeout_ms),
            conn_semaphore: Arc::new(Semaphore::new(pool_size)),
        }
    }
}

#[async_trait]
impl PageFetcher for CdpRenderer {
    async fn fetch(
        &self,
        url: &str,
        _headers: &HashMap<String, String>,
        wait_for_ms: Option<u64>,
    ) -> CrwResult<FetchResult> {
        // Overall hard timeout: page_timeout + wait_for + connect overhead.
        // Prevents indefinite hangs even if individual CDP commands complete.
        let wait_dur = Duration::from_millis(wait_for_ms.unwrap_or(DEFAULT_JS_WAIT_MS));
        let overall_timeout = self.page_timeout + wait_dur + FETCH_OVERHEAD;

        tokio::time::timeout(overall_timeout, self.fetch_with_ws(url, wait_for_ms))
            .await
            .map_err(|_| CrwError::Timeout(overall_timeout.as_millis() as u64))?
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn supports_js(&self) -> bool {
        true
    }

    async fn is_available(&self) -> bool {
        // Try connecting and running a minimal CDP command to verify the
        // renderer can actually process requests, not just accept connections.
        let ws = match tokio::time::timeout(WS_CONNECT_TIMEOUT, connect_async(&self.ws_url)).await {
            Ok(Ok((ws, _))) => ws,
            _ => return false,
        };
        let (mut write, mut read) = ws.split();
        let check = cdp_send_recv(
            &mut write,
            &mut read,
            "Browser.getVersion",
            serde_json::json!({}),
            None,
            Duration::from_secs(5),
        )
        .await;
        close_ws(&mut write).await;
        check.is_ok()
    }
}

/// Detect LightPanda/Chrome navigation error pages.
/// These are rendered as normal HTML instead of CDP-level errors.
fn detect_navigation_error(html: &str) -> Option<String> {
    // Only check short HTML — real pages are much larger than error pages.
    if html.len() > 2000 {
        return None;
    }
    let lower = html.to_lowercase();
    if lower.contains("navigation failed") || lower.contains("navigationerror") {
        // Try to extract the reason
        if let Some(start) = lower.find("reason:") {
            let after = &lower[start + 7..];
            let reason = after
                .split(&['<', '\n'][..])
                .next()
                .unwrap_or("unknown")
                .trim();
            return Some(reason.to_string());
        }
        return Some("unknown".to_string());
    }
    None
}

impl CdpRenderer {
    /// Inner fetch with WebSocket lifecycle management.
    async fn fetch_with_ws(&self, url: &str, wait_for_ms: Option<u64>) -> CrwResult<FetchResult> {
        let start = Instant::now();

        // Limit concurrent WebSocket connections to pool_size.
        let _permit = self
            .conn_semaphore
            .acquire()
            .await
            .map_err(|_| CrwError::RendererError("Connection pool closed".into()))?;

        // Open a fresh WebSocket connection per request
        let (ws, _) = tokio::time::timeout(WS_CONNECT_TIMEOUT, connect_async(&self.ws_url))
            .await
            .map_err(|_| CrwError::Timeout(WS_CONNECT_TIMEOUT.as_millis() as u64))?
            .map_err(|e| CrwError::RendererError(format!("CDP connect: {e}")))?;

        let (mut write, mut read) = ws.split();

        let result = self
            .fetch_inner(&mut write, &mut read, url, wait_for_ms)
            .await;

        // Always close WebSocket, even on error
        close_ws(&mut write).await;

        let (html, status_code) = result?;

        if html.is_empty() {
            return Err(CrwError::RendererError(
                "Empty HTML from CDP renderer".into(),
            ));
        }

        if let Some(reason) = detect_navigation_error(&html) {
            return Err(CrwError::RendererError(format!(
                "Navigation failed: {reason}"
            )));
        }

        Ok(FetchResult {
            url: url.to_string(),
            status_code,
            html,
            rendered_with: Some(self.name.clone()),
            elapsed_ms: start.elapsed().as_millis() as u64,
            warning: None,
        })
    }

    async fn fetch_inner(
        &self,
        write: &mut WsWrite,
        read: &mut WsRead,
        url: &str,
        wait_for_ms: Option<u64>,
    ) -> CrwResult<(String, u16)> {
        // 1. Create a blank target so navigation events can be observed reliably.
        let create_result = cdp_send_recv(
            write,
            read,
            "Target.createTarget",
            serde_json::json!({ "url": "about:blank" }),
            None,
            self.page_timeout,
        )
        .await?;

        let target_id = create_result
            .get("targetId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CrwError::RendererError(format!("No targetId: {create_result}")))?
            .to_string();

        // 2. Attach to target
        let attach_result = match cdp_send_recv(
            write,
            read,
            "Target.attachToTarget",
            serde_json::json!({ "targetId": &target_id, "flatten": true }),
            None,
            self.page_timeout,
        )
        .await
        {
            Ok(result) => result,
            Err(err) => {
                close_target(write, read, &target_id).await;
                return Err(err);
            }
        };

        let session_id = match attach_result
            .get("sessionId")
            .and_then(|value| value.as_str())
        {
            Some(value) => value.to_string(),
            None => {
                close_target(write, read, &target_id).await;
                return Err(CrwError::RendererError(
                    "CDP attach did not return sessionId".into(),
                ));
            }
        };

        for method in ["Page.enable", "Network.enable", "Runtime.enable"] {
            if let Err(err) = cdp_send_recv(
                write,
                read,
                method,
                serde_json::json!({}),
                Some(&session_id),
                self.page_timeout,
            )
            .await
            {
                close_target(write, read, &target_id).await;
                return Err(err);
            }
        }

        let navigate_result = match cdp_send_recv(
            write,
            read,
            "Page.navigate",
            serde_json::json!({ "url": url }),
            Some(&session_id),
            self.page_timeout,
        )
        .await
        {
            Ok(result) => result,
            Err(err) => {
                close_target(write, read, &target_id).await;
                return Err(err);
            }
        };

        if let Some(error_text) = navigate_result
            .get("errorText")
            .and_then(|value| value.as_str())
        {
            close_target(write, read, &target_id).await;
            return Err(CrwError::RendererError(format!(
                "Navigation failed: {error_text}"
            )));
        }

        let status_code = match wait_for_page_ready(read, &session_id, self.page_timeout).await {
            Ok(status) => status,
            Err(err) => {
                close_target(write, read, &target_id).await;
                return Err(err);
            }
        };

        // 3. Wait for additional JS work requested by the caller.
        let wait = wait_for_ms.unwrap_or(DEFAULT_JS_WAIT_MS);
        tokio::time::sleep(Duration::from_millis(wait)).await;

        // 4. Get rendered HTML
        let eval_result = match cdp_send_recv(
            write,
            read,
            "Runtime.evaluate",
            serde_json::json!({
                "expression": "document.documentElement.outerHTML",
                "returnByValue": true
            }),
            Some(&session_id),
            self.page_timeout,
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                close_target(write, read, &target_id).await;
                return Err(e);
            }
        };

        let html = eval_result
            .get("result")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // 5. Cleanup target
        close_target(write, read, &target_id).await;

        Ok((html, status_code))
    }
}
