//! `CrwBrowse` — the rmcp server that exposes Phase 1 tools (`goto`, `tree`).
//!
//! Walking skeleton: a single default session is created lazily on the first
//! tool call. Multi-session + `session.new`/`session.close` tools land in
//! Phase 2 (see ROADMAP).

use std::sync::Arc;
use std::time::{Duration, Instant};

use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crw_renderer::cdp_conn::{CdpConnection, CdpEvent};

use crate::errors::ErrorResponse;
use crate::response::ToolResponse;
use crate::session::{BrowserSession, SessionRegistry};
use crate::snapshot;

/// Upper bound for per-call `timeout_ms` — anything larger gets clamped. Keeps
/// a rogue client from pinning a CDP session for hours on a typoed value. When
/// the clamp fires, a `warnings` entry is added to the response so the caller
/// knows the effective value differs from what they asked for.
const MAX_TIMEOUT_MS: u64 = 120_000;
/// Upper bound for `tree` output size — the AX tree of a big page can blow up
/// LLM context if not capped. Clamped silently to the cap; callers see the
/// effective truncation through `data.node_count > tree line count`.
const MAX_TREE_NODES: u32 = 5_000;
/// Maximum byte length of a `goto` URL. RFC doesn't mandate a hard limit but
/// 2048 covers every practical real-world URL and blocks megabyte-sized
/// prompt-injection payloads that would otherwise burn CPU in `url::Url::parse`
/// and flood logs.
const MAX_URL_LEN: usize = 2048;
/// URL schemes accepted by `goto`. Everything else — including but not limited
/// to `file://`, `data:`, `javascript:`, `chrome:`, `about:`, `blob:`,
/// `view-source:`, `intent:`, `filesystem:`, `chrome-extension:`, `ws://`,
/// `ftp://` — is rejected with `InvalidArgs` so a prompt-injection payload
/// can't pivot the browser onto the local filesystem, an app-launch URI, or
/// an in-page code-execution protocol. The allowlist is explicit (not
/// blacklist) per OWASP fail-closed guidance.
const ALLOWED_GOTO_SCHEMES: &[&str] = &["http", "https"];

/// Startup configuration for the server.
#[derive(Debug, Clone)]
pub struct BrowseConfig {
    pub ws_url: String,
    pub page_timeout: Duration,
}

impl Default for BrowseConfig {
    fn default() -> Self {
        Self {
            ws_url: "ws://localhost:9222".to_string(),
            page_timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Clone)]
pub struct CrwBrowse {
    config: Arc<BrowseConfig>,
    registry: Arc<SessionRegistry>,
    default_session: Arc<RwLock<Option<Arc<BrowserSession>>>>,
    #[allow(dead_code)] // read by the #[tool_handler] generated glue
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct GotoInput {
    /// URL to navigate to.
    pub url: String,
    /// Navigation timeout in milliseconds (default: 30000).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct TreeInput {
    /// Maximum number of nodes to include in the output (default: 500).
    #[serde(default)]
    pub max_nodes: Option<u32>,
}

#[derive(Debug, Serialize)]
struct GotoData {
    status: u16,
}

#[derive(Debug, Serialize)]
struct TreeData {
    node_count: usize,
    tree: String,
}

#[tool_router]
impl CrwBrowse {
    pub fn new(config: BrowseConfig) -> Self {
        Self {
            config: Arc::new(config),
            registry: Arc::new(SessionRegistry::new()),
            default_session: Arc::new(RwLock::new(None)),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Navigate the browser to the given URL and wait for the page to load. \
                       Only `http` and `https` schemes are accepted; any other scheme \
                       (file://, data:, javascript:, blob:, etc.) returns `INVALID_ARGS`. \
                       Creates a default session on first call. Response includes `session` \
                       (4-char token), `url`, `data.status` (HTTP status, 0 if the network \
                       event was missed — see `warnings`), and `elapsed_ms`. `timeout_ms` \
                       is capped at 120000; values above are clamped and a `warnings` \
                       entry reports the clamp."
    )]
    pub async fn goto(
        &self,
        Parameters(input): Parameters<GotoInput>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();

        if let Err(msg) = validate_goto_url(&input.url) {
            // Log the scheme only, never the full URL: the URL is attacker-
            // controlled via an LLM and could contain auth tokens, injection
            // payloads, or multi-megabyte garbage. Even the pre-parse scheme
            // slice is bounded to 32 bytes so a percent-encoded or malformed
            // prefix (e.g. `http%3A//` or a 65KB non-URL with no ':' at all)
            // can't balloon the log line.
            let raw_prefix = input.url.split(':').next().unwrap_or("unknown");
            let scheme_for_log = &raw_prefix[..raw_prefix.len().min(32)];
            tracing::warn!(
                scheme = scheme_for_log,
                "goto rejected — disallowed scheme or malformed url"
            );
            return Ok(err_result(&ErrorResponse::new(
                crate::errors::ErrorCode::InvalidArgs,
                msg,
            )));
        }

        let (timeout, timeout_clamped) = clamp_timeout(input.timeout_ms, self.config.page_timeout);

        let session = match self.ensure_default_session().await {
            Ok(s) => s,
            Err(e) => {
                return Ok(err_result(&ErrorResponse::new(
                    crate::errors::ErrorCode::BrowserUnavailable,
                    format!("failed to open CDP connection: {e}"),
                )));
            }
        };

        let cdp_sid = match session.ensure_attached(timeout).await {
            Ok(sid) => sid,
            Err(e) => {
                return Ok(err_result(&ErrorResponse::new(
                    crate::errors::ErrorCode::CdpError,
                    format!("failed to attach target: {e}"),
                )));
            }
        };

        // Subscribe before navigating so we don't miss Page.loadEventFired.
        let events_rx = session.conn.subscribe();

        let navigate = session
            .conn
            .send_recv(
                "Page.navigate",
                serde_json::json!({ "url": input.url }),
                Some(&cdp_sid),
                timeout,
            )
            .await;

        if let Err(e) = navigate {
            return Ok(err_result(&ErrorResponse::new(
                crate::errors::ErrorCode::NavBlocked,
                format!("Page.navigate failed: {e}"),
            )));
        }

        let load = wait_for_load(events_rx, &cdp_sid, timeout).await;
        session.set_last_url(&input.url).await;

        let status = load.unwrap_or(0);
        let mut payload = ToolResponse::new(
            &session.short_id,
            Some(input.url.clone()),
            GotoData { status },
        )
        .with_navigated(true)
        .with_elapsed_ms(started.elapsed().as_millis() as u64);
        if load.is_none() {
            payload = payload.with_warning(
                "HTTP status unknown (no Network.responseReceived Document event before load)",
            );
        }
        if timeout_clamped {
            // Tell the caller their timeout request was shortened. Without
            // this they'd silently see a Timeout error for a value they
            // believed was larger than it actually was.
            payload = payload.with_warning(format!(
                "timeout_ms clamped to {MAX_TIMEOUT_MS} ms (server-side cap)"
            ));
        }
        Ok(ok_result(&payload))
    }

    #[tool(
        description = "Snapshot the current page as an indented accessibility tree. \
                       Each line is `[nodeId] role: name`, with 2-space indentation to \
                       show parent/child structure. `nodeId` tokens are stable within \
                       one snapshot and will be accepted by future interaction tools \
                       (`click`, `fill_form`, etc.). Reduce `max_nodes` for large pages \
                       to save tokens; values above 5000 are clamped. Requires a prior \
                       `goto` call."
    )]
    pub async fn tree(
        &self,
        Parameters(input): Parameters<TreeInput>,
    ) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        let (max_nodes_u32, max_nodes_clamped) = clamp_max_nodes(input.max_nodes);
        let max_nodes = max_nodes_u32 as usize;

        let Some(session) = self.default_session.read().await.clone() else {
            return Ok(err_result(&ErrorResponse::new(
                crate::errors::ErrorCode::NotFound,
                "no session yet — call `goto` first",
            )));
        };
        let Some(cdp_sid) = session.cdp_session_id().await else {
            return Ok(err_result(&ErrorResponse::new(
                crate::errors::ErrorCode::NotFound,
                "session has no attached target — call `goto` first",
            )));
        };

        let ax = match session
            .conn
            .send_recv(
                "Accessibility.getFullAXTree",
                serde_json::json!({}),
                Some(&cdp_sid),
                self.config.page_timeout,
            )
            .await
        {
            Ok(v) => v,
            Err(e) => {
                return Ok(err_result(&ErrorResponse::new(
                    crate::errors::ErrorCode::CdpError,
                    format!("Accessibility.getFullAXTree failed: {e}"),
                )));
            }
        };

        let nodes = ax.get("nodes").cloned().unwrap_or(serde_json::Value::Null);
        let node_count = nodes.as_array().map(|a| a.len()).unwrap_or(0);
        let rendered = snapshot::render_compact(&nodes, max_nodes);

        let mut payload = ToolResponse::new(
            &session.short_id,
            session.last_url().await,
            TreeData {
                node_count,
                tree: rendered,
            },
        )
        .with_elapsed_ms(started.elapsed().as_millis() as u64);
        if max_nodes_clamped {
            payload = payload.with_warning(format!(
                "max_nodes clamped to {MAX_TREE_NODES} (server-side cap)"
            ));
        }
        if node_count > max_nodes {
            payload = payload.with_warning(format!(
                "tree truncated: {node_count} nodes in AX, showing first {max_nodes}"
            ));
        }
        Ok(ok_result(&payload))
    }

    async fn ensure_default_session(
        &self,
    ) -> Result<Arc<BrowserSession>, crw_core::error::CrwError> {
        if let Some(s) = self.default_session.read().await.clone()
            && !s.is_closing.load(std::sync::atomic::Ordering::SeqCst)
        {
            return Ok(s);
        }
        let mut slot = self.default_session.write().await;
        if let Some(s) = slot.clone()
            && !s.is_closing.load(std::sync::atomic::Ordering::SeqCst)
        {
            return Ok(s);
        }
        let conn = CdpConnection::connect(&self.config.ws_url, Duration::from_secs(10)).await?;
        let session = self.registry.insert(Arc::new(conn))?;
        *slot = Some(session.clone());
        Ok(session)
    }
}

#[tool_handler]
impl ServerHandler for CrwBrowse {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::from_build_env())
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
            .with_instructions(
                "Interactive browser automation over CDP. Call `goto` to navigate, \
             then `tree` to inspect the rendered accessibility tree."
                    .to_string(),
            )
    }
}

fn ok_result<T: serde::Serialize>(resp: &ToolResponse<T>) -> CallToolResult {
    CallToolResult::success(vec![Content::text(resp.to_json())])
}

fn err_result(err: &ErrorResponse) -> CallToolResult {
    let mut result = CallToolResult::success(vec![Content::text(err.to_json())]);
    result.is_error = Some(true);
    result
}

/// Validates a `goto` target URL. Returns the caller-facing error message when
/// the URL is too long, malformed, or uses a disallowed scheme. The whitelist
/// is explicit (`http`/`https` only) because an LLM caller is a prompt-
/// injection-reachable surface — we do not want to let it pivot to `file://`
/// for local disclosure, `data:` for HTML-smuggled XSS, or `javascript:` for
/// in-page code execution.
///
/// The parse-error branch deliberately does NOT echo the input URL back into
/// the error message: URLs may contain auth tokens and the error propagates
/// through MCP's response envelope to logs and client UIs. The scheme branch
/// echoes only the parsed scheme (a short, bounded, sanitized string).
fn validate_goto_url(url: &str) -> Result<(), String> {
    if url.is_empty() {
        return Err("empty url".to_string());
    }
    if url.len() > MAX_URL_LEN {
        return Err(format!("url exceeds maximum length of {MAX_URL_LEN} bytes"));
    }
    let parsed = url::Url::parse(url).map_err(|e| format!("invalid url: {e}"))?;
    let scheme = parsed.scheme();
    if !ALLOWED_GOTO_SCHEMES.contains(&scheme) {
        return Err(format!(
            "scheme {scheme:?} not allowed — goto accepts http or https only"
        ));
    }
    Ok(())
}

/// Apply `MAX_TIMEOUT_MS` to a caller-supplied `timeout_ms` and return both the
/// effective `Duration` and a flag indicating whether clamping occurred. Pulled
/// out as a pure function so it can be unit-tested without a CDP connection.
fn clamp_timeout(timeout_ms: Option<u64>, default: Duration) -> (Duration, bool) {
    // Defence-in-depth: the cap applies even to the config-sourced default.
    // `BrowseConfig::page_timeout` is `pub` so an embedder could construct an
    // oversized default; without this floor the `None` branch would silently
    // bypass `MAX_TIMEOUT_MS`.
    let cap = Duration::from_millis(MAX_TIMEOUT_MS);
    match timeout_ms {
        Some(ms) => {
            let clamped = ms > MAX_TIMEOUT_MS;
            let effective = ms.min(MAX_TIMEOUT_MS);
            (Duration::from_millis(effective), clamped)
        }
        None => (default.min(cap), false),
    }
}

/// Apply `MAX_TREE_NODES` to a caller-supplied `max_nodes`, defaulting to 500
/// when unset. Returns both the effective value and a clamp flag. Pure.
fn clamp_max_nodes(max_nodes: Option<u32>) -> (u32, bool) {
    match max_nodes {
        Some(n) => {
            let clamped = n > MAX_TREE_NODES;
            (n.min(MAX_TREE_NODES), clamped)
        }
        None => (500, false),
    }
}

/// Waits until either `Page.loadEventFired` arrives or `timeout` elapses, and
/// returns the HTTP status from the first `Network.responseReceived` event
/// with `type: "Document"` that matches our session. Returns `None` if no
/// Document response was observed, regardless of which terminator fires.
///
/// Splits `Lagged` from `Closed` on the broadcast receiver — on lag we retry,
/// but on closure we return whatever status we have so far rather than
/// spinning to the deadline.
async fn wait_for_load(
    mut events: tokio::sync::broadcast::Receiver<CdpEvent>,
    cdp_session_id: &str,
    timeout: Duration,
) -> Option<u16> {
    use tokio::sync::broadcast::error::RecvError;
    let deadline = tokio::time::Instant::now() + timeout;
    let mut status: Option<u16> = None;
    loop {
        let recv = tokio::time::timeout_at(deadline, events.recv()).await;
        match recv {
            Err(_) => return status,
            Ok(Err(RecvError::Closed)) => return status,
            Ok(Err(RecvError::Lagged(n))) => {
                tracing::warn!(
                    lagged = n,
                    "wait_for_load broadcast lagged — may have missed page events"
                );
                continue;
            }
            Ok(Ok(ev)) => {
                if ev.session_id.as_deref() != Some(cdp_session_id) {
                    continue;
                }
                if ev.method == "Network.responseReceived" {
                    let is_doc = ev
                        .params
                        .get("type")
                        .and_then(|v| v.as_str())
                        .is_some_and(|v| v == "Document");
                    if is_doc {
                        status = ev
                            .params
                            .get("response")
                            .and_then(|r| r.get("status"))
                            .and_then(|s| s.as_f64())
                            .map(|s| s as u16)
                            .or(status);
                    }
                } else if ev.method == "Page.loadEventFired" {
                    return status;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_goto_url_accepts_http_and_https() {
        assert!(validate_goto_url("http://example.com").is_ok());
        assert!(validate_goto_url("https://example.com/path?q=1").is_ok());
    }

    #[test]
    fn validate_goto_url_rejects_dangerous_schemes() {
        // Exhaustive list of the scheme families a prompt-injection payload
        // could reach for on a modern browser. Any new allowlist member in
        // the future should force one of these to move to the accepted list
        // intentionally, not silently via a dropped test.
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "chrome://settings",
            "about:blank",
            "ftp://example.com",
            "ws://localhost:9222",
            "wss://localhost:9222",
            "blob:https://example.com/some-uuid",
            "view-source:https://example.com",
            "intent://example.com/#Intent;scheme=https;end",
            "filesystem:https://example.com/file",
            "chrome-extension://abcdef/page.html",
        ] {
            let err = validate_goto_url(bad).expect_err(bad);
            assert!(
                err.contains("not allowed"),
                "expected scheme rejection for {bad:?}, got {err}"
            );
        }
    }

    #[test]
    fn validate_goto_url_normalizes_mixed_case_scheme() {
        // url::Url::parse normalizes the scheme to lowercase per WHATWG. These
        // asserts pin that guarantee so a future parser swap can't silently
        // open a bypass.
        assert!(validate_goto_url("HTTPS://example.com").is_ok());
        assert!(validate_goto_url("Http://example.com").is_ok());
        let err = validate_goto_url("JavaScript:alert(1)").expect_err("js mixed-case");
        assert!(err.contains("not allowed"), "got: {err}");
        let err = validate_goto_url("FILE:///etc/passwd").expect_err("file mixed-case");
        assert!(err.contains("not allowed"), "got: {err}");
    }

    #[test]
    fn validate_goto_url_rejects_percent_encoded_scheme() {
        // Per RFC 3986 a scheme must be [a-zA-Z][a-zA-Z0-9+.-]* — percent
        // escapes in the scheme position are malformed and must be rejected
        // as parse errors, not unwrapped back into `http`.
        assert!(validate_goto_url("%68ttp://example.com").is_err());
        assert!(validate_goto_url("ht%74ps://example.com").is_err());
    }

    #[test]
    fn validate_goto_url_rejects_malformed() {
        assert!(validate_goto_url("not a url").is_err());
        assert!(validate_goto_url("").is_err());
    }

    #[test]
    fn validate_goto_url_rejects_oversize() {
        // Construct a valid-scheme URL whose total length exceeds MAX_URL_LEN.
        let long_path = "a".repeat(MAX_URL_LEN);
        let url = format!("https://example.com/{long_path}");
        let err = validate_goto_url(&url).expect_err("oversize");
        assert!(err.contains("maximum length"), "got: {err}");
    }

    #[test]
    fn validate_goto_url_does_not_echo_bad_url_in_error() {
        // Security property: a caller-supplied URL that fails to parse must
        // not appear verbatim in the error message (it may contain auth
        // tokens or injection payloads). Only the parser's own diagnostic
        // is allowed through.
        let secret = "https://attacker.example.com/?token=sk-super-secret";
        // Force a parse failure by corrupting the URL.
        let bad = format!("{secret}\x00\x00\x00not-parsable");
        if let Err(msg) = validate_goto_url(&bad) {
            assert!(
                !msg.contains("sk-super-secret"),
                "error message must not echo the bad URL: {msg}"
            );
        }
    }

    #[test]
    fn clamp_timeout_preserves_in_range() {
        let (d, clamped) = clamp_timeout(Some(30_000), Duration::from_secs(10));
        assert_eq!(d, Duration::from_millis(30_000));
        assert!(!clamped);
    }

    #[test]
    fn clamp_timeout_caps_excessive() {
        let (d, clamped) = clamp_timeout(Some(999_999_999), Duration::from_secs(10));
        assert_eq!(d, Duration::from_millis(MAX_TIMEOUT_MS));
        assert!(clamped);
    }

    #[test]
    fn clamp_timeout_none_uses_default() {
        let default = Duration::from_secs(45);
        let (d, clamped) = clamp_timeout(None, default);
        assert_eq!(d, default);
        assert!(!clamped);
    }

    #[test]
    fn clamp_max_nodes_default_when_unset() {
        let (n, clamped) = clamp_max_nodes(None);
        assert_eq!(n, 500);
        assert!(!clamped);
    }

    #[test]
    fn clamp_max_nodes_preserves_in_range() {
        let (n, clamped) = clamp_max_nodes(Some(1000));
        assert_eq!(n, 1000);
        assert!(!clamped);
    }

    #[test]
    fn clamp_max_nodes_caps_excessive() {
        let (n, clamped) = clamp_max_nodes(Some(u32::MAX));
        assert_eq!(n, MAX_TREE_NODES);
        assert!(clamped);
    }

    // Boundary: exactly at the cap must pass through as NOT clamped. A naive
    // `>=` comparison would regress here without tripping any of the "wildly
    // oversized" tests above.
    #[test]
    fn clamp_timeout_at_exact_cap_is_not_clamped() {
        let (d, clamped) = clamp_timeout(Some(MAX_TIMEOUT_MS), Duration::from_secs(10));
        assert_eq!(d, Duration::from_millis(MAX_TIMEOUT_MS));
        assert!(!clamped, "exact cap should pass through");
    }

    #[test]
    fn clamp_max_nodes_at_exact_cap_is_not_clamped() {
        let (n, clamped) = clamp_max_nodes(Some(MAX_TREE_NODES));
        assert_eq!(n, MAX_TREE_NODES);
        assert!(!clamped, "exact cap should pass through");
    }

    // Defence-in-depth: an embedder-supplied default longer than the cap must
    // be floored to `MAX_TIMEOUT_MS`, even though the `None` branch doesn't
    // report clamping (the caller didn't ask for an oversize value).
    #[test]
    fn clamp_timeout_none_floors_oversized_default() {
        let oversized = Duration::from_millis(MAX_TIMEOUT_MS * 10);
        let (d, clamped) = clamp_timeout(None, oversized);
        assert_eq!(d, Duration::from_millis(MAX_TIMEOUT_MS));
        assert!(!clamped);
    }
}
