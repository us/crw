//! `goto` — navigate the browser to a URL.

use std::time::{Duration, Instant};

use rmcp::{ErrorData as McpError, model::CallToolResult, schemars};
use serde::{Deserialize, Serialize};

use crw_renderer::cdp_conn::CdpEvent;

use crate::errors::{ErrorCode, ErrorResponse};
use crate::response::ToolResponse;
use crate::server::CrwBrowse;
use crate::tools::common::{
    ALLOWED_GOTO_SCHEMES, MAX_TIMEOUT_MS, MAX_URL_LEN, clamp_timeout, err_result, ok_result,
};

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct GotoInput {
    /// URL to navigate to.
    pub url: String,
    /// Navigation timeout in milliseconds (default: 30000).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct GotoData {
    pub status: u16,
}

/// What precedes the first ':' in a rejected url, bounded to 32 bytes, for
/// logging. The url is attacker-controlled via an LLM and could carry auth
/// tokens, injection payloads or multi-megabyte garbage, so the query and path
/// never reach the log; for anything url-shaped this is just the scheme. The
/// bound keeps a percent-encoded or malformed prefix (`http%3A//`, or a 65KB
/// non-url with no ':' at all) from ballooning the log line. Note an input with
/// no ':' has no scheme to isolate, so it is logged as-is up to the bound.
///
/// The bound lands on a char boundary: the url is arbitrary text, so byte 32 can
/// fall inside a multibyte char, and slicing there would turn a rejected url
/// into a panic on the way to logging it.
fn scheme_for_log(url: &str) -> &str {
    let raw_prefix = url.split(':').next().unwrap_or("unknown");
    &raw_prefix[..raw_prefix.floor_char_boundary(32)]
}

pub async fn handle(server: &CrwBrowse, input: GotoInput) -> Result<CallToolResult, McpError> {
    let started = Instant::now();

    if let Err(msg) = validate_goto_url(&input.url) {
        tracing::warn!(
            scheme = scheme_for_log(&input.url),
            "goto rejected — disallowed scheme or malformed url"
        );
        return Ok(err_result(&ErrorResponse::new(ErrorCode::InvalidArgs, msg)));
    }

    let (timeout, timeout_clamped) = clamp_timeout(input.timeout_ms, server.config().page_timeout);

    let session = match server.ensure_default_session().await {
        Ok(s) => s,
        Err(e) => {
            return Ok(err_result(&ErrorResponse::new(
                ErrorCode::BrowserUnavailable,
                format!("failed to open CDP connection: {e}"),
            )));
        }
    };

    let cdp_sid = match session.ensure_attached(timeout).await {
        Ok(sid) => sid,
        Err(e) => {
            return Ok(err_result(&ErrorResponse::new(
                ErrorCode::CdpError,
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
            ErrorCode::NavBlocked,
            format!("Page.navigate failed: {e}"),
        )));
    }

    let load = wait_for_load(events_rx, &cdp_sid, timeout).await;
    session.set_last_url(&input.url).await;
    // Drop any `@e<N>` refs collected by a prior `tree` — they point at
    // the previous document's backend node IDs, which Chromium will
    // happily resolve to detached/stale nodes. Forcing the next ref-based
    // tool call to fail with `NODE_STALE` makes the LLM re-snapshot.
    session.clear_ref_map().await;

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
        payload = payload.with_warning(format!(
            "timeout_ms clamped to {MAX_TIMEOUT_MS} ms (server-side cap)"
        ));
    }
    Ok(ok_result(&payload))
}

/// Validates a `goto` target URL. Returns the caller-facing error message when
/// the URL is too long, malformed, or uses a disallowed scheme.
pub(crate) fn validate_goto_url(url: &str) -> Result<(), String> {
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
    crw_core::url_safety::validate_safe_url(&parsed)?;
    Ok(())
}

async fn validate_goto_url_resolved(url: &str) -> Result<(), String> {
    validate_goto_url(url)?;
    let parsed = url::Url::parse(url).map_err(|e| format!("invalid url: {e}"))?;
    tokio::time::timeout(
        Duration::from_secs(2),
        crw_core::url_safety::validate_safe_url_resolved(&parsed),
    )
    .await
    .map_err(|_| "DNS validation timed out".to_string())?
}

pub(crate) async fn enable_outbound_guard(
    conn: &crw_renderer::cdp_conn::CdpConnection,
    cdp_session_id: &str,
    timeout: Duration,
) -> crw_core::error::CrwResult<()> {
    conn.send_recv(
        "Fetch.enable",
        serde_json::json!({
            "patterns": [
                { "urlPattern": "*", "requestStage": "Request" }
            ]
        }),
        Some(cdp_session_id),
        timeout,
    )
    .await
    .map(|_| ())
}

pub(crate) async fn run_outbound_guard(
    conn: std::sync::Arc<crw_renderer::cdp_conn::CdpConnection>,
    mut events: tokio::sync::broadcast::Receiver<CdpEvent>,
    cdp_session_id: &str,
) {
    use tokio::sync::broadcast::error::RecvError;
    let concurrency = std::sync::Arc::new(tokio::sync::Semaphore::new(32));
    let cmd_timeout = Duration::from_secs(2);
    loop {
        let ev = match events.recv().await {
            Ok(ev) => ev,
            Err(RecvError::Closed) => return,
            Err(RecvError::Lagged(_)) => continue,
        };
        if ev.session_id.as_deref() != Some(cdp_session_id) || ev.method != "Fetch.requestPaused" {
            continue;
        }
        let request_id = ev
            .params
            .get("requestId")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if request_id.is_empty() {
            continue;
        }
        let permit = match concurrency.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                let _ = conn
                    .send_recv(
                        "Fetch.failRequest",
                        serde_json::json!({
                            "requestId": request_id,
                            "errorReason": "BlockedByClient",
                        }),
                        Some(cdp_session_id),
                        cmd_timeout,
                    )
                    .await;
                continue;
            }
        };
        let req_url = ev
            .params
            .get("request")
            .and_then(|r| r.get("url"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let request_id = request_id.to_string();
        let req_url = req_url.to_string();
        let conn = conn.clone();
        let cdp_session_id = cdp_session_id.to_string();
        tokio::spawn(async move {
            let _permit = permit;
            let method = if validate_goto_url_resolved(&req_url).await.is_ok() {
                "Fetch.continueRequest"
            } else {
                "Fetch.failRequest"
            };
            let params = if method == "Fetch.continueRequest" {
                serde_json::json!({ "requestId": request_id })
            } else {
                serde_json::json!({ "requestId": request_id, "errorReason": "BlockedByClient" })
            };
            let _ = conn
                .send_recv(method, params, Some(&cdp_session_id), cmd_timeout)
                .await;
        });
    }
}

/// Waits until either `Page.loadEventFired` arrives or `timeout` elapses, and
/// returns the HTTP status from the first `Network.responseReceived` event
/// with `type: "Document"` that matches our session.
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
                            .and_then(|s| {
                                // Defence-in-depth: CDP shouldn't return
                                // out-of-range, but `s as u16` truncation
                                // would wrap negative or > u16::MAX values
                                // into bogus codes (e.g. -1 → 65535).
                                if (0.0..=65_535.0).contains(&s) {
                                    Some(s as u16)
                                } else {
                                    None
                                }
                            })
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
        assert!(validate_goto_url("HTTPS://example.com").is_ok());
        assert!(validate_goto_url("Http://example.com").is_ok());
        let err = validate_goto_url("JavaScript:alert(1)").expect_err("js mixed-case");
        assert!(err.contains("not allowed"), "got: {err}");
        let err = validate_goto_url("FILE:///etc/passwd").expect_err("file mixed-case");
        assert!(err.contains("not allowed"), "got: {err}");
    }

    #[test]
    fn validate_goto_url_rejects_percent_encoded_scheme() {
        assert!(validate_goto_url("%68ttp://example.com").is_err());
        assert!(validate_goto_url("ht%74ps://example.com").is_err());
    }

    #[test]
    fn validate_goto_url_rejects_internal_networks() {
        for bad in [
            "http://127.0.0.1",
            "http://10.0.0.1",
            "http://169.254.169.254/latest/meta-data/",
            "http://[::1]/",
        ] {
            assert!(validate_goto_url(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn validate_goto_url_rejects_malformed() {
        assert!(validate_goto_url("not a url").is_err());
        assert!(validate_goto_url("").is_err());
    }

    #[test]
    fn validate_goto_url_rejects_oversize() {
        let long_path = "a".repeat(MAX_URL_LEN);
        let url = format!("https://example.com/{long_path}");
        let err = validate_goto_url(&url).expect_err("oversize");
        assert!(err.contains("maximum length"), "got: {err}");
    }

    #[test]
    fn validate_goto_url_does_not_echo_bad_url_in_error() {
        let secret = "https://attacker.example.com/?token=sk-super-secret";
        let bad = format!("{secret}\x00\x00\x00not-parsable");
        if let Err(msg) = validate_goto_url(&bad) {
            assert!(
                !msg.contains("sk-super-secret"),
                "error message must not echo the bad URL: {msg}"
            );
        }
    }

    /// The rejection path logs a bounded prefix of the url. The url is arbitrary
    /// LLM-supplied text, so that bound must not split a multibyte char — and the
    /// "no ':' at all" case the bound exists for is exactly where it lands
    /// mid-char.
    ///
    /// The char has to be placed deliberately: a repeated char only straddles
    /// byte 32 when its width does not divide 32, so `"é".repeat(n)` (2 bytes)
    /// and `"😀".repeat(n)` (4) land on a boundary and would pass unpatched. Pad
    /// with ASCII instead so each width crosses the bound at every interior
    /// offset, and assert the straddle so a fixture cannot go quietly vacuous.
    #[test]
    fn rejected_multibyte_url_prefix_is_bounded_on_a_char_boundary() {
        for (c, offset) in [
            ('é', 1),
            ('ハ', 1),
            ('ハ', 2),
            ('😀', 1),
            ('😀', 2),
            ('😀', 3),
        ] {
            // No ':' anywhere, so the whole url is the prefix to be bounded.
            let mut url = "a".repeat(32 - offset);
            url.push(c);
            url.push_str("tail");
            assert!(
                !url.is_char_boundary(32),
                "{c:?}/{offset}: byte 32 must be mid-char"
            );
            assert!(validate_goto_url(&url).is_err(), "{url} must be rejected");

            let logged = scheme_for_log(&url);
            assert!(logged.len() <= 32, "{c:?}/{offset}: bound must hold");
            assert!(url.starts_with(logged), "{c:?}/{offset}: must be a prefix");
        }
    }

    #[test]
    fn scheme_for_log_keeps_the_scheme_of_an_ordinary_url() {
        assert_eq!(scheme_for_log("https://example.com/a"), "https");
        assert_eq!(scheme_for_log("no-colon-at-all"), "no-colon-at-all");
    }
}
