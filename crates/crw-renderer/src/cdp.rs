use async_trait::async_trait;
use crw_core::error::{CrwError, CrwResult};
use crw_core::types::FetchResult;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{OnceCell, Semaphore, broadcast};
use tokio_tungstenite::connect_async;

use crate::cdp_conn::{CdpConnection, CdpEvent};
use crate::traits::PageFetcher;

/// Timeout for WebSocket connect handshake.
const WS_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
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
/// Maximum time to poll for content stability when a loading placeholder
/// is detected after the initial wait.
const CONTENT_STABILITY_MAX_MS: u64 = 6000;
/// Interval between content-stability polls.
const CONTENT_STABILITY_TICK_MS: u64 = 500;

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
                    drop(ws);
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
    let conf_stripped = configured
        .trim_start_matches("ws://")
        .trim_start_matches("wss://");
    let conf_host_port = conf_stripped.split('/').next().unwrap_or(conf_stripped);

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

async fn close_target(conn: &CdpConnection, target_id: &str) {
    let _ = conn
        .send_recv(
            "Target.closeTarget",
            serde_json::json!({ "targetId": target_id }),
            None,
            TARGET_CLOSE_TIMEOUT,
        )
        .await;
}

/// Consume events from `events` until `Page.loadEventFired` (returns the main
/// document status) or a fatal event arrives. Uses `main_document_status`
/// captured from `Network.responseReceived` when available.
async fn wait_for_page_ready(
    mut events: broadcast::Receiver<CdpEvent>,
    session_id: &str,
    timeout: Duration,
) -> CrwResult<u16> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut main_document_status: Option<u16> = None;

    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Err(_) => return Err(CrwError::Timeout(timeout.as_millis() as u64)),
            Ok(Err(broadcast::error::RecvError::Closed)) => {
                return Err(CrwError::RendererError(
                    "CDP event channel closed before load".into(),
                ));
            }
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Ok(ev)) => {
                if ev.session_id.as_deref() != Some(session_id) {
                    continue;
                }
                match ev.method.as_str() {
                    "Network.responseReceived" => {
                        let is_document = ev
                            .params
                            .get("type")
                            .and_then(|v| v.as_str())
                            .is_some_and(|v| v == "Document");
                        if is_document {
                            main_document_status = ev
                                .params
                                .get("response")
                                .and_then(|r| r.get("status"))
                                .and_then(|s| s.as_f64())
                                .map(|s| s as u16)
                                .or(main_document_status);
                        }
                    }
                    "Page.loadEventFired" => {
                        return Ok(main_document_status.unwrap_or(200));
                    }
                    "Inspector.targetCrashed" => {
                        return Err(CrwError::RendererError(
                            "Target crashed during render".into(),
                        ));
                    }
                    _ => {}
                }
            }
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
        // Overall hard timeout: page_timeout + wait_for + challenge retry budget
        // + content-stability budget (auto-mode only) + overhead. Challenge retries
        // can add up to CHALLENGE_MAX_RETRIES * CHALLENGE_POLL_INTERVAL_MS.
        let wait_dur = Duration::from_millis(wait_for_ms.unwrap_or(DEFAULT_JS_WAIT_MS));
        let challenge_budget =
            Duration::from_millis(CHALLENGE_POLL_INTERVAL_MS * u64::from(CHALLENGE_MAX_RETRIES));
        let stability_budget = if wait_for_ms.is_none() {
            Duration::from_millis(CONTENT_STABILITY_MAX_MS)
        } else {
            Duration::ZERO
        };
        let overall_timeout =
            self.page_timeout + wait_dur + challenge_budget + stability_budget + FETCH_OVERHEAD;

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
        let ws_url = match self.resolve_ws_url().await {
            Ok(url) => url,
            Err(_) => return false,
        };
        let conn = match CdpConnection::connect(&ws_url, WS_CONNECT_TIMEOUT).await {
            Ok(conn) => conn,
            Err(_) => return false,
        };
        let check = conn
            .send_recv(
                "Browser.getVersion",
                serde_json::json!({}),
                None,
                Duration::from_secs(5),
            )
            .await;
        conn.close().await;
        check.is_ok()
    }
}

/// Check if HTML looks like a Cloudflare/anti-bot challenge page.
fn is_challenge_page(html: &str) -> bool {
    if html.len() > 50_000 {
        return false;
    }
    let lower = html.to_lowercase();
    lower.contains("just a moment")
        || lower.contains("cf-browser-verification")
        || lower.contains("cf-challenge-running")
        || lower.contains("challenge-platform")
        || (lower.contains("challenge") && lower.contains("cloudflare"))
        || lower.contains("attention required")
}

/// Detect LightPanda/Chrome navigation error pages.
fn detect_navigation_error(html: &str) -> Option<String> {
    if html.len() > 2000 {
        return None;
    }
    let lower = html.to_lowercase();
    if lower.contains("navigation failed") || lower.contains("navigationerror") {
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

        let ws_url = self.resolve_ws_url().await?;
        let conn = CdpConnection::connect(&ws_url, WS_CONNECT_TIMEOUT).await?;

        let result = self.fetch_inner(&conn, url, wait_for_ms).await;

        conn.close().await;

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

    async fn eval_html(
        conn: &CdpConnection,
        session_id: &str,
        timeout: Duration,
    ) -> CrwResult<String> {
        let eval_result = conn
            .send_recv(
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

    /// Poll `document.documentElement.outerHTML` at a fixed interval until the
    /// rendered HTML stabilises and no longer looks like a loading placeholder,
    /// or until the stability budget is exhausted.
    async fn poll_until_content_stable(
        conn: &CdpConnection,
        session_id: &str,
        timeout: Duration,
    ) -> CrwResult<String> {
        let deadline = Instant::now() + Duration::from_millis(CONTENT_STABILITY_MAX_MS);
        let mut prev_len: u64 = 0;
        let mut stable_ticks: u32 = 0;
        let mut last_html = String::new();

        while Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(CONTENT_STABILITY_TICK_MS)).await;
            let html = Self::eval_html(conn, session_id, timeout).await?;
            let len = html.len() as u64;
            let placeholder_gone = !crate::detector::looks_like_loading_placeholder(&html);
            if is_content_stable(prev_len, len, placeholder_gone) {
                stable_ticks += 1;
                if stable_ticks >= 2 {
                    return Ok(html);
                }
            } else {
                stable_ticks = 0;
            }
            prev_len = len;
            last_html = html;
        }
        Ok(last_html)
    }

    async fn fetch_inner(
        &self,
        conn: &CdpConnection,
        url: &str,
        wait_for_ms: Option<u64>,
    ) -> CrwResult<(String, u16)> {
        // 1. Create a blank target so navigation events can be observed reliably.
        let create_result = conn
            .send_recv(
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
        let attach_result = match conn
            .send_recv(
                "Target.attachToTarget",
                serde_json::json!({ "targetId": &target_id, "flatten": true }),
                None,
                self.page_timeout,
            )
            .await
        {
            Ok(result) => result,
            Err(err) => {
                close_target(conn, &target_id).await;
                return Err(err);
            }
        };

        let session_id = match attach_result
            .get("sessionId")
            .and_then(|value| value.as_str())
        {
            Some(value) => value.to_string(),
            None => {
                close_target(conn, &target_id).await;
                return Err(CrwError::RendererError(
                    "CDP attach did not return sessionId".into(),
                ));
            }
        };

        for method in ["Page.enable", "Network.enable", "Runtime.enable"] {
            if let Err(err) = conn
                .send_recv(
                    method,
                    serde_json::json!({}),
                    Some(&session_id),
                    self.page_timeout,
                )
                .await
            {
                close_target(conn, &target_id).await;
                return Err(err);
            }
        }

        // Inject stealth scripts before navigation so they run on every new document.
        if let Err(err) = conn
            .send_recv(
                "Page.addScriptToEvaluateOnNewDocument",
                serde_json::json!({ "source": STEALTH_JS }),
                Some(&session_id),
                self.page_timeout,
            )
            .await
        {
            close_target(conn, &target_id).await;
            return Err(err);
        }

        // Subscribe to events BEFORE navigating so we don't miss loadEventFired.
        let events_rx = conn.subscribe();

        let navigate_result = match conn
            .send_recv(
                "Page.navigate",
                serde_json::json!({ "url": url }),
                Some(&session_id),
                self.page_timeout,
            )
            .await
        {
            Ok(result) => result,
            Err(err) => {
                close_target(conn, &target_id).await;
                return Err(err);
            }
        };

        if let Some(error_text) = navigate_result
            .get("errorText")
            .and_then(|value| value.as_str())
        {
            close_target(conn, &target_id).await;
            return Err(CrwError::RendererError(format!(
                "Navigation failed: {error_text}"
            )));
        }

        let status_code = match wait_for_page_ready(events_rx, &session_id, self.page_timeout).await
        {
            Ok(status) => status,
            Err(err) => {
                close_target(conn, &target_id).await;
                return Err(err);
            }
        };

        // 3. Wait for initial JS work requested by the caller.
        let wait = wait_for_ms.unwrap_or(DEFAULT_JS_WAIT_MS);
        tokio::time::sleep(Duration::from_millis(wait)).await;

        // 4. Get rendered HTML.
        let mut html = match Self::eval_html(conn, &session_id, self.page_timeout).await {
            Ok(h) => h,
            Err(e) => {
                close_target(conn, &target_id).await;
                return Err(e);
            }
        };

        // 4b. SPA loading placeholder → poll for content stability.
        if wait_for_ms.is_none() && crate::detector::looks_like_loading_placeholder(&html) {
            tracing::info!(
                url,
                "Loading placeholder detected, polling for content stability"
            );
            match Self::poll_until_content_stable(conn, &session_id, self.page_timeout).await {
                Ok(stable) => html = stable,
                Err(e) => tracing::warn!("Content stability polling failed: {e}"),
            }
        }

        // 5. Challenge retry loop for Cloudflare/anti-bot interstitials.
        if is_challenge_page(&html) {
            tracing::info!(url, "Challenge page detected, waiting for auto-resolve");
            for attempt in 1..=CHALLENGE_MAX_RETRIES {
                tokio::time::sleep(Duration::from_millis(CHALLENGE_POLL_INTERVAL_MS)).await;

                html = match Self::eval_html(conn, &session_id, self.page_timeout).await {
                    Ok(h) => h,
                    Err(e) => {
                        tracing::warn!(attempt, "Challenge retry eval failed: {e}");
                        close_target(conn, &target_id).await;
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
        close_target(conn, &target_id).await;

        Ok((html, status_code))
    }
}

/// Pure decision: does the current poll tick indicate the rendered page has
/// stabilised? Returns `false` on the first tick (`prev_len == 0`) so that at
/// least two observations are required. `placeholder_gone` must be `true`
/// (the rendered HTML no longer looks like a loading placeholder).
///
/// Size tolerance is 5% of `prev_len` with a 500-byte floor, so noise from
/// small DOM updates (timestamps, counters) does not reset stability.
fn is_content_stable(prev_len: u64, curr_len: u64, placeholder_gone: bool) -> bool {
    if prev_len == 0 || !placeholder_gone {
        return false;
    }
    let tolerance = (prev_len / 20).max(500);
    curr_len.abs_diff(prev_len) <= tolerance
}

#[cfg(test)]
mod tests {
    use super::is_content_stable;

    #[test]
    fn first_tick_never_stable() {
        assert!(!is_content_stable(0, 0, true));
        assert!(!is_content_stable(0, 10_000, true));
    }

    #[test]
    fn identical_sizes_are_stable_when_placeholder_gone() {
        assert!(is_content_stable(5_000, 5_000, true));
    }

    #[test]
    fn placeholder_still_present_blocks_stability() {
        assert!(!is_content_stable(5_000, 5_000, false));
    }

    #[test]
    fn small_delta_within_tolerance_is_stable() {
        assert!(is_content_stable(10_000, 10_400, true));
    }

    #[test]
    fn large_delta_outside_tolerance_is_unstable() {
        assert!(!is_content_stable(10_000, 12_000, true));
    }

    #[test]
    fn small_page_uses_500_byte_floor() {
        assert!(is_content_stable(100, 450, true));
    }

    #[test]
    fn shrink_past_tolerance_is_unstable() {
        assert!(!is_content_stable(10_000, 5_000, true));
    }
}
