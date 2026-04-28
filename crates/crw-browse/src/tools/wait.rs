//! `wait` — block until a page-side condition is met or the timeout fires.
//!
//! Three modes, picked from the input:
//! - `selector` (CSS) → poll `document.querySelector` every 100ms until a
//!   match appears (or `condition: visible` requires the match to also be
//!   non-`display:none`).
//! - `condition: load` → wait for `Page.loadEventFired`.
//! - `condition: networkidle` → wait until 500ms of network silence after
//!   the buffer's last entry. Cheap-and-cheerful: relies on the
//!   per-session network ring buffer that the listener task maintains.
//!
//! Polling intentionally uses `Runtime.evaluate` rather than `DOM.querySelector`
//! to keep the path identical to the inspection tools and to side-step
//! `DOM`-domain quirks in Lightpanda.

use std::time::{Duration, Instant};

use rmcp::{ErrorData as McpError, model::CallToolResult, schemars};
use serde::{Deserialize, Serialize};

use crate::errors::{ErrorCode, ErrorResponse};
use crate::response::ToolResponse;
use crate::server::CrwBrowse;
use crate::tools::common::{
    EvalOutcome, MAX_TIMEOUT_MS, clamp_timeout, err_result, no_session_err, no_target_err,
    ok_result, runtime_evaluate,
};

const POLL_INTERVAL: Duration = Duration::from_millis(100);
const NETWORK_IDLE_WINDOW: Duration = Duration::from_millis(500);
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct WaitInput {
    /// CSS selector to wait for. Mutually exclusive with `condition` and `ms`.
    #[serde(default)]
    pub selector: Option<String>,
    /// `visible` (default for selector path; element must also be in the
    /// rendered tree, i.e. not `display:none`), `present` (any DOM match),
    /// `load` (wait for `Page.loadEventFired`), or `networkidle` (wait
    /// for 500ms of network silence).
    #[serde(default)]
    pub condition: Option<String>,
    /// Fixed duration sleep in ms. Mutually exclusive with `selector` and
    /// `condition`. Capped at `MAX_WAIT_MS` (60_000) — for longer pauses
    /// chain multiple `wait{ms}` calls. Useful for "settle" pauses after
    /// triggering a debounced handler when no observable selector exists.
    #[serde(default)]
    pub ms: Option<u64>,
    /// Per-call timeout. Defaults to 5000ms; capped at 120000.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// Cap on `ms` mode so a misuse can't pin a session for hours. Longer pauses
/// can be composed by calling `wait{ms}` repeatedly.
const MAX_WAIT_MS: u64 = 60_000;

#[derive(Debug, Serialize)]
pub struct WaitData {
    /// What was waited on (`selector`, `load`, `networkidle`).
    pub waited_for: String,
    /// Time spent waiting in ms.
    pub waited_ms: u64,
}

/// Resolved wait mode produced by [`validate_wait_args`]. The string variant
/// owns the selector so the validation can run without borrowing `input`.
#[derive(Debug, PartialEq, Eq)]
enum WaitMode {
    SelectorVisible(String),
    SelectorPresent(String),
    Load,
    NetworkIdle,
    /// Fixed-duration sleep, in ms. Already validated against `MAX_WAIT_MS`.
    Sleep(u64),
}

/// Pure input validation. Hoisted out of `handle` so unit tests can hit the
/// bad-condition arms without spinning up a live session.
#[allow(clippy::result_large_err)]
fn validate_wait_args(
    selector: Option<&str>,
    condition: Option<&str>,
    ms: Option<u64>,
) -> Result<WaitMode, ErrorResponse> {
    // Reject empty strings explicitly. `Some("")` would otherwise fall through
    // to the `Some(sel)` arms and produce `document.querySelector("")`, which
    // is a syntax error the LLM has to debug from a CDP exception. Same for
    // condition: `condition: ""` should be a clear "supply a value" message,
    // not "unknown condition '' ".
    if let Some(sel) = selector
        && sel.trim().is_empty()
    {
        return Err(ErrorResponse::new(
            ErrorCode::InvalidArgs,
            "selector must not be empty — pass a CSS selector like '#login' or 'button.primary'",
        ));
    }
    if let Some(c) = condition
        && c.trim().is_empty()
    {
        return Err(ErrorResponse::new(
            ErrorCode::InvalidArgs,
            "condition must not be empty — expected one of: visible, present, load, networkidle",
        ));
    }
    // `ms` is mutually exclusive with the observable wait modes — combining
    // them would silently pick one and ignore the others, which is exactly
    // the kind of silent footgun this tool's "explicit error" stance avoids.
    if let Some(n) = ms {
        if selector.is_some() || condition.is_some() {
            return Err(ErrorResponse::new(
                ErrorCode::InvalidArgs,
                "ms cannot be combined with selector or condition — pick one wait mode",
            ));
        }
        if n == 0 {
            return Err(ErrorResponse::new(ErrorCode::InvalidArgs, "ms must be > 0"));
        }
        if n > MAX_WAIT_MS {
            return Err(ErrorResponse::new(
                ErrorCode::InvalidArgs,
                format!(
                    "ms must be <= {MAX_WAIT_MS} — chain multiple wait{{ms}} calls for longer pauses"
                ),
            ));
        }
        return Ok(WaitMode::Sleep(n));
    }
    let cond_lc = condition.map(str::to_lowercase);
    match (selector, cond_lc.as_deref()) {
        (Some(sel), None | Some("visible")) => Ok(WaitMode::SelectorVisible(sel.to_string())),
        (Some(sel), Some("present")) => Ok(WaitMode::SelectorPresent(sel.to_string())),
        (Some(_), Some(other)) => Err(ErrorResponse::new(
            ErrorCode::InvalidArgs,
            format!("condition '{other}' invalid with selector — expected 'visible' or 'present'"),
        )),
        (None, Some("load")) => Ok(WaitMode::Load),
        (None, Some("networkidle")) => Ok(WaitMode::NetworkIdle),
        (None, Some(other)) => Err(ErrorResponse::new(
            ErrorCode::InvalidArgs,
            format!("condition '{other}' requires no selector — expected 'load' or 'networkidle'"),
        )),
        (None, None) => Err(ErrorResponse::new(
            ErrorCode::InvalidArgs,
            "wait requires `selector`, `condition` ∈ {load, networkidle}, or `ms`",
        )),
    }
}

pub async fn handle(server: &CrwBrowse, input: WaitInput) -> Result<CallToolResult, McpError> {
    let started = Instant::now();
    if let Some(0) = input.timeout_ms {
        return Ok(err_result(&ErrorResponse::new(
            ErrorCode::InvalidArgs,
            "timeout_ms must be > 0",
        )));
    }
    let (timeout, timeout_clamped) = clamp_timeout(input.timeout_ms, DEFAULT_TIMEOUT);

    // Validate up-front so a bad `condition` is rejected with the same
    // error whether or not a session exists.
    let mode = match validate_wait_args(
        input.selector.as_deref(),
        input.condition.as_deref(),
        input.ms,
    ) {
        Ok(m) => m,
        Err(e) => return Ok(err_result(&e)),
    };

    // `ms` mode does not need a live session — fixed sleeps work even when
    // no page is open. Run it before the session/target lookups so callers
    // can use it as a `goto`-free pause primitive.
    if let WaitMode::Sleep(n) = mode {
        tokio::time::sleep(Duration::from_millis(n)).await;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        // Synthetic short_id/url for the no-session case — keeps the
        // response shape identical to the other wait modes so callers
        // don't need to special-case the payload.
        let (short_id, url) = if let Some(session) = server.default_session_get().await {
            (session.short_id.clone(), session.last_url().await)
        } else {
            ("none".to_string(), None)
        };
        let mut payload = ToolResponse::new(
            &short_id,
            url,
            WaitData {
                waited_for: "ms".to_string(),
                waited_ms: elapsed_ms,
            },
        )
        .with_elapsed_ms(elapsed_ms);
        if timeout_clamped {
            payload = payload.with_warning(format!(
                "timeout_ms clamped to {MAX_TIMEOUT_MS} ms (server-side cap)"
            ));
        }
        return Ok(ok_result(&payload));
    }

    let Some(session) = server.default_session_get().await else {
        return Ok(err_result(&no_session_err()));
    };
    let Some(cdp_sid) = session.cdp_session_id().await else {
        return Ok(err_result(&no_target_err()));
    };

    let outcome = match mode {
        WaitMode::SelectorVisible(sel) => wait_selector(&session, &cdp_sid, &sel, true, timeout)
            .await
            .map(|_| "selector".to_string()),
        WaitMode::SelectorPresent(sel) => wait_selector(&session, &cdp_sid, &sel, false, timeout)
            .await
            .map(|_| "selector".to_string()),
        WaitMode::Load => wait_load(&session, &cdp_sid, timeout)
            .await
            .map(|_| "load".to_string()),
        WaitMode::NetworkIdle => wait_network_idle(&session, timeout)
            .await
            .map(|_| "networkidle".to_string()),
        WaitMode::Sleep(_) => unreachable!("ms mode handled before session lookup"),
    };

    match outcome {
        Err(e) => Ok(err_result(&e)),
        Ok(waited_for) => {
            let elapsed_ms = started.elapsed().as_millis() as u64;
            let mut payload = ToolResponse::new(
                &session.short_id,
                session.last_url().await,
                WaitData {
                    waited_for,
                    waited_ms: elapsed_ms,
                },
            )
            .with_elapsed_ms(elapsed_ms);
            if timeout_clamped {
                payload = payload.with_warning(format!(
                    "timeout_ms clamped to {MAX_TIMEOUT_MS} ms (server-side cap)"
                ));
            }
            Ok(ok_result(&payload))
        }
    }
}

async fn wait_selector(
    session: &crate::session::BrowserSession,
    cdp_sid: &str,
    selector: &str,
    require_visible: bool,
    timeout: Duration,
) -> Result<(), ErrorResponse> {
    let sel_json = serde_json::to_string(selector).unwrap_or_else(|_| "\"\"".into());
    let expr = if require_visible {
        // `getClientRects().length` is the standard test for "in the rendered
        // tree" — `display:none` and detached elements have zero rects.
        format!(
            r#"(() => {{
                const el = document.querySelector({sel_json});
                if (!el) return false;
                if (typeof el.getClientRects === 'function' && el.getClientRects().length === 0) return false;
                return true;
            }})()"#
        )
    } else {
        format!("!!document.querySelector({sel_json})")
    };

    let deadline = Instant::now() + timeout;
    // Hard cap on each evaluate so a single hung poll can't pin us
    // indefinitely; the actual budget is the smaller of this cap and the
    // time remaining until the deadline, computed per-iteration so a poll
    // can never run longer than the caller's `timeout_ms` allows.
    const PER_POLL_CAP: Duration = Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ErrorResponse::new(
                ErrorCode::Timeout,
                format!(
                    "selector {:?} did not appear within {} ms",
                    selector,
                    timeout.as_millis()
                ),
            ));
        }
        let per_poll = PER_POLL_CAP.min(remaining);
        match runtime_evaluate(session, cdp_sid, &expr, per_poll).await? {
            EvalOutcome::Threw(msg) => {
                return Err(ErrorResponse::new(
                    ErrorCode::InvalidExpression,
                    format!("selector poll threw: {msg}"),
                ));
            }
            EvalOutcome::Ok { value, .. } => {
                if value.and_then(|v| v.as_bool()).unwrap_or(false) {
                    return Ok(());
                }
            }
        }
        // Sleep no longer than the remaining budget — the next iteration's
        // top-of-loop check then surfaces the timeout instead of waking up
        // past the deadline and running another wasted poll.
        let after_eval = deadline.saturating_duration_since(Instant::now());
        if after_eval.is_zero() {
            continue;
        }
        tokio::time::sleep(POLL_INTERVAL.min(after_eval)).await;
    }
}

async fn wait_load(
    session: &crate::session::BrowserSession,
    cdp_session_id: &str,
    timeout: Duration,
) -> Result<(), ErrorResponse> {
    // Pin the deadline to the *start* of this call. Both the readyState
    // probe and the broadcast loop must fit inside `timeout` total —
    // computing the deadline AFTER the probe would let a 500ms probe +
    // a 600ms requested timeout produce ~1100ms wall time.
    let deadline = tokio::time::Instant::now() + timeout;
    // Subscribe BEFORE probing readyState so we can't miss a
    // `loadEventFired` that arrives between the probe and the subscribe.
    let mut rx = session.conn.subscribe();
    // Past-fired race: if `Page.loadEventFired` already fired before
    // this call started (common when `goto` returned and the agent
    // immediately calls `wait` for `load`), no future event will ever
    // fire and we'd hang until timeout. Probe `document.readyState`
    // first; if it's already `"complete"`, return success without
    // waiting. The probe is also re-run after each `Lagged` so a load
    // event lost in the broadcast lag window doesn't cause a false
    // timeout.
    let probe_timeout = Duration::from_millis(500).min(timeout);
    if probe_ready(session, cdp_session_id, probe_timeout).await {
        return Ok(());
    }
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Err(_) => {
                return Err(ErrorResponse::new(
                    ErrorCode::Timeout,
                    format!(
                        "Page.loadEventFired did not arrive within {} ms",
                        timeout.as_millis()
                    ),
                ));
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                return Err(ErrorResponse::new(
                    ErrorCode::CdpError,
                    "CDP event channel closed while waiting for load",
                ));
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {
                // The broadcast buffer overflowed and we may have
                // missed `Page.loadEventFired` itself. Re-probe
                // readyState before re-entering recv() — if the page
                // is already complete, return success rather than
                // wait for another event that will never come.
                if probe_ready(session, cdp_session_id, probe_timeout).await {
                    return Ok(());
                }
                continue;
            }
            Ok(Ok(ev)) => {
                if ev.session_id.as_deref() == Some(cdp_session_id)
                    && ev.method == "Page.loadEventFired"
                {
                    return Ok(());
                }
            }
        }
    }
}

/// `true` iff `document.readyState === 'complete'` evaluates successfully
/// to `true` within `probe_timeout`. Any error (CDP failure, evaluate
/// throw, non-bool result) returns `false` — the caller falls back to
/// the broadcast-loop path so a transient probe failure doesn't decide
/// the wait outcome on its own.
async fn probe_ready(
    session: &crate::session::BrowserSession,
    cdp_session_id: &str,
    probe_timeout: Duration,
) -> bool {
    matches!(
        runtime_evaluate(
            session,
            cdp_session_id,
            "document.readyState === 'complete'",
            probe_timeout,
        )
        .await,
        Ok(EvalOutcome::Ok {
            value: Some(serde_json::Value::Bool(true)),
            ..
        })
    )
}

async fn wait_network_idle(
    session: &crate::session::BrowserSession,
    timeout: Duration,
) -> Result<(), ErrorResponse> {
    // Watches the per-session monotonic Network event counter (see
    // `BrowserSession::network_event_count`). If the counter holds steady for
    // `NETWORK_IDLE_WINDOW`, network is considered idle. We deliberately use
    // the monotonic counter rather than `network_buffer.len()` — the buffer is
    // a bounded ring (cap 500), so on a chatty page the visible length can
    // pin at the cap while events keep flowing, which the buffer-length
    // version would misread as idle.
    let deadline = Instant::now() + timeout;
    let mut last_count = session.network_event_count();
    let mut idle_since = Instant::now();
    loop {
        if Instant::now() >= deadline {
            return Err(ErrorResponse::new(
                ErrorCode::Timeout,
                format!("network did not idle within {} ms", timeout.as_millis()),
            ));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
        let count_now = session.network_event_count();
        if count_now == last_count {
            if idle_since.elapsed() >= NETWORK_IDLE_WINDOW {
                return Ok(());
            }
        } else {
            last_count = count_now;
            idle_since = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::{BrowseConfig, CrwBrowse};

    fn err_text(result: &CallToolResult) -> String {
        assert_eq!(result.is_error, Some(true), "expected is_error=true");
        result
            .content
            .first()
            .and_then(|c| c.raw.as_text().map(|t| t.text.clone()))
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn wait_rejects_zero_timeout() {
        let server = CrwBrowse::new(BrowseConfig::default());
        let res = handle(
            &server,
            WaitInput {
                selector: Some("#x".into()),
                condition: None,
                ms: None,
                timeout_ms: Some(0),
            },
        )
        .await
        .expect("handle");
        let body = err_text(&res);
        assert!(
            body.contains("timeout_ms must be > 0"),
            "expected timeout-zero rejection, got: {body}"
        );
    }

    #[test]
    fn validate_accepts_selector_with_default_condition() {
        let m = validate_wait_args(Some("#x"), None, None).expect("ok");
        assert_eq!(m, WaitMode::SelectorVisible("#x".into()));
    }

    #[test]
    fn validate_accepts_present_with_selector() {
        let m = validate_wait_args(Some("#x"), Some("present"), None).expect("ok");
        assert_eq!(m, WaitMode::SelectorPresent("#x".into()));
    }

    #[test]
    fn validate_accepts_load_and_networkidle_alone() {
        assert_eq!(
            validate_wait_args(None, Some("load"), None).expect("ok"),
            WaitMode::Load
        );
        assert_eq!(
            validate_wait_args(None, Some("NetworkIdle"), None).expect("ok"),
            WaitMode::NetworkIdle
        );
    }

    #[test]
    fn validate_rejects_unknown_condition_with_selector() {
        let err = validate_wait_args(Some("#x"), Some("bogus"), None).expect_err("err");
        assert!(
            err.message.contains("invalid with selector"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn validate_rejects_unknown_condition_without_selector() {
        let err = validate_wait_args(None, Some("bogus"), None).expect_err("err");
        assert!(
            err.message.contains("requires no selector"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn validate_rejects_no_selector_no_condition() {
        let err = validate_wait_args(None, None, None).expect_err("err");
        assert!(err.message.contains("requires"), "got: {}", err.message);
    }

    #[tokio::test]
    async fn handle_returns_invalid_args_for_unknown_condition() {
        let server = CrwBrowse::new(BrowseConfig::default());
        let res = handle(
            &server,
            WaitInput {
                selector: Some("#x".into()),
                condition: Some("bogus".into()),
                ms: None,
                timeout_ms: Some(100),
            },
        )
        .await
        .expect("handle");
        let body = err_text(&res);
        assert!(
            body.contains("invalid with selector"),
            "expected condition rejection, got: {body}"
        );
    }

    #[test]
    fn validate_accepts_ms_alone() {
        let m = validate_wait_args(None, None, Some(250)).expect("ok");
        assert_eq!(m, WaitMode::Sleep(250));
    }

    #[test]
    fn validate_rejects_ms_with_selector() {
        let err = validate_wait_args(Some("#x"), None, Some(250)).expect_err("err");
        assert!(
            err.message.contains("cannot be combined"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn validate_rejects_ms_with_condition() {
        let err = validate_wait_args(None, Some("load"), Some(250)).expect_err("err");
        assert!(
            err.message.contains("cannot be combined"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn validate_rejects_zero_ms() {
        let err = validate_wait_args(None, None, Some(0)).expect_err("err");
        assert!(
            err.message.contains("ms must be > 0"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn validate_rejects_ms_over_cap() {
        let err = validate_wait_args(None, None, Some(MAX_WAIT_MS + 1)).expect_err("err");
        assert!(
            err.message.contains(&MAX_WAIT_MS.to_string()),
            "got: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn handle_ms_sleeps_without_session() {
        let server = CrwBrowse::new(BrowseConfig::default());
        let started = Instant::now();
        let res = handle(
            &server,
            WaitInput {
                selector: None,
                condition: None,
                ms: Some(50),
                timeout_ms: None,
            },
        )
        .await
        .expect("handle");
        assert_ne!(
            res.is_error,
            Some(true),
            "ms mode should not require a session"
        );
        assert!(
            started.elapsed() >= Duration::from_millis(50),
            "expected at least 50ms wait, took {:?}",
            started.elapsed()
        );
    }
}
