use async_trait::async_trait;
use crw_core::error::{CrwError, CrwResult};
use crw_core::types::FetchResult;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::traits::PageFetcher;

/// Lightweight CDP client that talks directly to any CDP-compatible browser
/// (LightPanda, Chrome, Playwright) via WebSocket.
pub struct CdpRenderer {
    name: String,
    ws_url: String,
    page_timeout: Duration,
}

#[derive(Deserialize, Debug)]
struct CdpMessage {
    id: Option<u64>,
    result: Option<serde_json::Value>,
    error: Option<serde_json::Value>,
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
    timeout: Duration,
) -> CrwResult<serde_json::Value> {
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    let req = serde_json::json!({ "id": id, "method": method, "params": params });

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

        if let Message::Text(text) = msg {
            if let Ok(resp) = serde_json::from_str::<CdpMessage>(&text) {
                if resp.id == Some(id) {
                    if let Some(err) = resp.error {
                        return Err(CrwError::RendererError(format!("CDP {method}: {err}")));
                    }
                    return Ok(resp.result.unwrap_or(serde_json::Value::Null));
                }
            }
        }
    }
}

/// Close WebSocket with a timeout to prevent hanging.
async fn close_ws(write: &mut WsWrite) {
    let _ = tokio::time::timeout(Duration::from_secs(3), write.close()).await;
}

impl CdpRenderer {
    pub fn new(name: &str, ws_url: &str, page_timeout_ms: u64) -> Self {
        Self {
            name: name.to_string(),
            ws_url: ws_url.to_string(),
            page_timeout: Duration::from_millis(page_timeout_ms),
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
        let start = Instant::now();

        // Open a fresh WebSocket connection per request
        let (ws, _) = tokio::time::timeout(Duration::from_secs(10), connect_async(&self.ws_url))
            .await
            .map_err(|_| CrwError::Timeout(10000))?
            .map_err(|e| CrwError::RendererError(format!("CDP connect: {e}")))?;

        let (mut write, mut read) = ws.split();

        // Use a closure-like pattern to ensure cleanup on error
        let result = self
            .fetch_inner(&mut write, &mut read, url, wait_for_ms)
            .await;

        // Always close WebSocket, even on error
        close_ws(&mut write).await;

        let html = result?;

        if html.is_empty() {
            return Err(CrwError::RendererError(
                "Empty HTML from CDP renderer".into(),
            ));
        }

        // Detect navigation error pages returned by LightPanda/Chrome.
        // LightPanda returns HTML like <h1>Navigation failed</h1><p>Reason: CouldntResolveHost</p>
        // instead of raising a CDP error.
        if let Some(reason) = detect_navigation_error(&html) {
            return Err(CrwError::RendererError(format!(
                "Navigation failed: {reason}"
            )));
        }

        Ok(FetchResult {
            url: url.to_string(),
            status_code: 200,
            html,
            rendered_with: Some(self.name.clone()),
            elapsed_ms: start.elapsed().as_millis() as u64,
        })
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn supports_js(&self) -> bool {
        true
    }

    async fn is_available(&self) -> bool {
        connect_async(&self.ws_url).await.is_ok()
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
            let after = &html[start + 7..];
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
    async fn fetch_inner(
        &self,
        write: &mut WsWrite,
        read: &mut WsRead,
        url: &str,
        wait_for_ms: Option<u64>,
    ) -> CrwResult<String> {
        // 1. Create target (navigate to URL)
        let create_result = cdp_send_recv(
            write,
            read,
            "Target.createTarget",
            serde_json::json!({ "url": url }),
            self.page_timeout,
        )
        .await?;

        let target_id = create_result
            .get("targetId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CrwError::RendererError(format!("No targetId: {create_result}")))?
            .to_string();

        // 2. Attach to target
        let _attach = cdp_send_recv(
            write,
            read,
            "Target.attachToTarget",
            serde_json::json!({ "targetId": &target_id, "flatten": true }),
            self.page_timeout,
        )
        .await?;

        // 3. Wait for JS
        let wait = wait_for_ms.unwrap_or(2000);
        tokio::time::sleep(Duration::from_millis(wait)).await;

        // 4. Get rendered HTML
        let eval_result = cdp_send_recv(
            write,
            read,
            "Runtime.evaluate",
            serde_json::json!({
                "expression": "document.documentElement.outerHTML",
                "returnByValue": true
            }),
            self.page_timeout,
        )
        .await?;

        let html = eval_result
            .get("result")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // 5. Cleanup target
        let _ = cdp_send_recv(
            write,
            read,
            "Target.closeTarget",
            serde_json::json!({ "targetId": &target_id }),
            Duration::from_secs(5),
        )
        .await;

        Ok(html)
    }
}
