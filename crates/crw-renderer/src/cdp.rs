use async_trait::async_trait;
use crw_core::error::{CrwError, CrwResult};
use crw_core::types::FetchResult;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::{OnceCell, Semaphore};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::traits::PageFetcher;

/// Timeout for WebSocket connect handshake.
const WS_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Timeout for graceful WebSocket close.
const WS_CLOSE_TIMEOUT: Duration = Duration::from_secs(3);
/// Extra overhead budget for the overall fetch timeout (on top of page_timeout + wait_for).
const FETCH_OVERHEAD: Duration = Duration::from_secs(30);
/// Timeout for the Target.closeTarget cleanup command.
const TARGET_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
/// Default JS wait time if not specified by the caller.
const DEFAULT_JS_WAIT_MS: u64 = 2000;
/// Maximum number of challenge retry attempts.
const CHALLENGE_MAX_RETRIES: u32 = 3;
/// Delay between challenge retry polls (ms).
const CHALLENGE_POLL_INTERVAL_MS: u64 = 3000;

/// JavaScript injected via `Page.addScriptToEvaluateOnNewDocument` before every
/// navigation to prevent headless browser detection by anti-bot systems.
const STEALTH_JS: &str = r#"
// 1. Hide navigator.webdriver (primary headless signal for Cloudflare)
Object.defineProperty(navigator, 'webdriver', { get: () => false });

// 2. Fake chrome runtime object (missing in headless)
if (!window.chrome) {
    window.chrome = { runtime: {}, loadTimes: function(){}, csi: function(){} };
}

// 3. Spoof plugins array (headless has 0 plugins)
Object.defineProperty(navigator, 'plugins', {
    get: () => {
        const arr = [
            { name: 'Chrome PDF Plugin', filename: 'internal-pdf-viewer' },
            { name: 'Chrome PDF Viewer', filename: 'mhjfbmdgcfjbbpaeojofohoefgiehjai' },
            { name: 'Native Client', filename: 'internal-nacl-plugin' },
        ];
        arr.item = (i) => arr[i];
        arr.namedItem = (n) => arr.find(p => p.name === n);
        arr.refresh = () => {};
        return arr;
    }
});

// 4. Spoof languages (headless sometimes returns empty)
Object.defineProperty(navigator, 'languages', { get: () => ['en-US', 'en'] });

// 5. Override permissions query to hide "denied" for notifications
const originalQuery = window.navigator.permissions.query.bind(window.navigator.permissions);
window.navigator.permissions.query = (params) =>
    params.name === 'notifications'
        ? Promise.resolve({ state: Notification.permission })
        : originalQuery(params);

// 6. Prevent detection via iframe contentWindow
const origHTMLElement = HTMLIFrameElement.prototype.__lookupGetter__('contentWindow');
if (origHTMLElement) {
    Object.defineProperty(HTMLIFrameElement.prototype, 'contentWindow', {
        get: function() {
            const w = origHTMLElement.call(this);
            if (w && !w.chrome) w.chrome = window.chrome;
            return w;
        }
    });
}

// 7. Fix broken toString for overridden functions (anti-detection fingerprinting)
const nativeToString = Function.prototype.toString;
const overrides = new Map();
const proxy = new Proxy(nativeToString, {
    apply(target, thisArg, args) {
        const override = overrides.get(thisArg);
        return override || nativeToString.call(thisArg);
    }
});
Function.prototype.toString = proxy;
overrides.set(Function.prototype.toString, 'function toString() { [native code] }');
"#;

/// Lightweight CDP client that talks directly to any CDP-compatible browser
/// (LightPanda, Chrome, Playwright) via WebSocket.
///
/// Uses a semaphore to limit concurrent connections to `pool_size`,
/// preventing connection storms under heavy concurrent crawl loads.
pub struct CdpRenderer {
    name: String,
    /// Base WS URL from config (e.g. "ws://chrome:9222/").
    /// For Chrome/Chromium, the actual browser WS URL includes a dynamic ID
    /// (e.g. "ws://chrome:9222/devtools/browser/<uuid>") and must be discovered
    /// at runtime via the /json/version HTTP endpoint.
    configured_ws_url: String,
    /// Lazily resolved browser-level WS URL (discovered from /json/version).
    resolved_ws_url: OnceCell<String>,
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
            configured_ws_url: ws_url.to_string(),
            resolved_ws_url: OnceCell::new(),
            page_timeout: Duration::from_millis(page_timeout_ms),
            conn_semaphore: Arc::new(Semaphore::new(pool_size)),
        }
    }

    /// Resolve the actual browser WebSocket URL.
    ///
    /// Chrome/Chromium expose a dynamic WS URL at `/json/version` that includes
    /// a per-session UUID (e.g. `ws://host:9222/devtools/browser/<uuid>`).
    /// LightPanda accepts connections directly on the base WS URL.
    ///
    /// This method tries the configured URL first. If it already contains a path
    /// (e.g. `/devtools/browser/...`), it's used as-is. Otherwise, it queries
    /// the `/json/version` HTTP endpoint to discover the real WS URL.
    async fn resolve_ws_url(&self) -> CrwResult<String> {
        self.resolved_ws_url
            .get_or_try_init(|| async {
                let configured = &self.configured_ws_url;

                // If URL already has a devtools path, use it directly.
                if configured.contains("/devtools/") {
                    return Ok(configured.clone());
                }

                // Try connecting directly first (works for LightPanda).
                if let Ok(Ok((ws, _))) =
                    tokio::time::timeout(Duration::from_secs(3), connect_async(configured)).await
                {
                    let (mut write, _) = ws.split();
                    close_ws(&mut write).await;
                    return Ok(configured.clone());
                }

                // Direct connection failed — try /json/version discovery (Chrome/Chromium).
                let http_url = configured
                    .replace("ws://", "http://")
                    .replace("wss://", "https://")
                    .trim_end_matches('/')
                    .to_string()
                    + "/json/version";

                tracing::info!(
                    renderer = self.name,
                    "Discovering browser WS URL from {http_url}"
                );

                // Send Host: localhost so browsers behind socat proxies
                // (e.g. chromedp/headless-shell) accept the request.
                let resp = reqwest::Client::new()
                    .get(&http_url)
                    .header("Host", "localhost")
                    .timeout(Duration::from_secs(5))
                    .send()
                    .await
                    .map_err(|e| {
                        CrwError::RendererError(format!(
                            "CDP discovery failed for {}: {e}",
                            self.name
                        ))
                    })?;

                let body: serde_json::Value = resp.json().await.map_err(|e| {
                    CrwError::RendererError(format!("CDP discovery parse error: {e}"))
                })?;

                let ws_url = body
                    .get("webSocketDebuggerUrl")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        CrwError::RendererError(format!(
                            "No webSocketDebuggerUrl in /json/version for {}",
                            self.name
                        ))
                    })?;

                // The URL from /json/version uses the container's internal hostname
                // (e.g. "ws://127.0.0.1:9222/..."). Replace it with our configured host.
                let resolved = rewrite_ws_host(ws_url, configured);
                tracing::info!(renderer = self.name, ws_url = %resolved, "Discovered browser WS URL");
                Ok(resolved)
            })
            .await
            .cloned()
    }
}

/// Rewrite the host:port of a WS URL to match the configured endpoint.
/// Chrome's /json/version returns "ws://127.0.0.1:9222/devtools/browser/..." but
/// from another container we need "ws://chrome:9222/devtools/browser/...".
fn rewrite_ws_host(discovered: &str, configured: &str) -> String {
    // Extract host:port from configured URL
    let conf_stripped = configured
        .trim_start_matches("ws://")
        .trim_start_matches("wss://");
    let conf_host_port = conf_stripped.split('/').next().unwrap_or(conf_stripped);

    // Extract path from discovered URL
    let disc_stripped = discovered
        .trim_start_matches("ws://")
        .trim_start_matches("wss://");
    let disc_path = disc_stripped
        .find('/')
        .map(|i| &disc_stripped[i..])
        .unwrap_or("/");

    let scheme = if configured.starts_with("wss://") {
        "wss://"
    } else {
        "ws://"
    };
    format!("{scheme}{conf_host_port}{disc_path}")
}

#[async_trait]
impl PageFetcher for CdpRenderer {
    async fn fetch(
        &self,
        url: &str,
        _headers: &HashMap<String, String>,
        wait_for_ms: Option<u64>,
    ) -> CrwResult<FetchResult> {
        // Overall hard timeout: page_timeout + wait_for + challenge retry budget + overhead.
        // Challenge retries can add up to CHALLENGE_MAX_RETRIES * CHALLENGE_POLL_INTERVAL_MS.
        let wait_dur = Duration::from_millis(wait_for_ms.unwrap_or(DEFAULT_JS_WAIT_MS));
        let challenge_budget =
            Duration::from_millis(CHALLENGE_POLL_INTERVAL_MS * u64::from(CHALLENGE_MAX_RETRIES));
        let overall_timeout = self.page_timeout + wait_dur + challenge_budget + FETCH_OVERHEAD;

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
        let ws_url = match self.resolve_ws_url().await {
            Ok(url) => url,
            Err(_) => return false,
        };
        let ws = match tokio::time::timeout(WS_CONNECT_TIMEOUT, connect_async(&ws_url)).await {
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

/// Check if HTML looks like a Cloudflare/anti-bot challenge page.
/// Returns true if we should wait and re-evaluate the page.
fn is_challenge_page(html: &str) -> bool {
    // Real content pages are large — challenge interstitials are small.
    if html.len() > 50_000 {
        return false;
    }
    let lower = html.to_lowercase();
    // Cloudflare JS challenge markers
    lower.contains("just a moment")
        || lower.contains("cf-browser-verification")
        || lower.contains("cf-challenge-running")
        || lower.contains("challenge-platform")
        || (lower.contains("challenge") && lower.contains("cloudflare"))
        // PerimeterX / generic
        || lower.contains("attention required")
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

        // Resolve the browser WS URL (discovered lazily on first use).
        let ws_url = self.resolve_ws_url().await?;

        // Open a fresh WebSocket connection per request
        let (ws, _) = tokio::time::timeout(WS_CONNECT_TIMEOUT, connect_async(&ws_url))
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
            content_type: None,
            raw_bytes: None,
            rendered_with: Some(self.name.clone()),
            elapsed_ms: start.elapsed().as_millis() as u64,
            warning: None,
        })
    }

    /// Evaluate JS in the page and return the string result.
    async fn eval_html(
        write: &mut WsWrite,
        read: &mut WsRead,
        session_id: &str,
        timeout: Duration,
    ) -> CrwResult<String> {
        let eval_result = cdp_send_recv(
            write,
            read,
            "Runtime.evaluate",
            serde_json::json!({
                "expression": "document.documentElement.outerHTML",
                "returnByValue": true
            }),
            Some(session_id),
            timeout,
        )
        .await?;

        Ok(eval_result
            .get("result")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string())
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

        // Inject stealth scripts before navigation so they run on every new document.
        // This prevents headless detection by anti-bot systems (Cloudflare, PerimeterX, etc.).
        if let Err(err) = cdp_send_recv(
            write,
            read,
            "Page.addScriptToEvaluateOnNewDocument",
            serde_json::json!({ "source": STEALTH_JS }),
            Some(&session_id),
            self.page_timeout,
        )
        .await
        {
            close_target(write, read, &target_id).await;
            return Err(err);
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

        // 3. Wait for initial JS work requested by the caller.
        let wait = wait_for_ms.unwrap_or(DEFAULT_JS_WAIT_MS);
        tokio::time::sleep(Duration::from_millis(wait)).await;

        // 4. Get rendered HTML.
        let mut html = match Self::eval_html(write, read, &session_id, self.page_timeout).await {
            Ok(h) => h,
            Err(e) => {
                close_target(write, read, &target_id).await;
                return Err(e);
            }
        };

        // 5. Challenge retry loop: if we see a Cloudflare/anti-bot interstitial,
        //    wait and re-evaluate — non-interactive JS challenges auto-resolve in ~5s.
        if is_challenge_page(&html) {
            tracing::info!(url, "Challenge page detected, waiting for auto-resolve");
            for attempt in 1..=CHALLENGE_MAX_RETRIES {
                tokio::time::sleep(Duration::from_millis(CHALLENGE_POLL_INTERVAL_MS)).await;

                html = match Self::eval_html(write, read, &session_id, self.page_timeout).await {
                    Ok(h) => h,
                    Err(e) => {
                        tracing::warn!(attempt, "Challenge retry eval failed: {e}");
                        close_target(write, read, &target_id).await;
                        return Err(e);
                    }
                };

                if !is_challenge_page(&html) {
                    tracing::info!(url, attempt, "Challenge cleared");
                    break;
                }
                tracing::debug!(url, attempt, "Challenge still active, retrying");
            }
        }

        // 6. Cleanup target.
        close_target(write, read, &target_id).await;

        Ok((html, status_code))
    }
}
