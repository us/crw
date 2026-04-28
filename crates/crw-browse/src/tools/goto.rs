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

pub async fn handle(server: &CrwBrowse, input: GotoInput) -> Result<CallToolResult, McpError> {
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
    Ok(())
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
}
