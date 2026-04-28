//! Shared helpers and limits used by every tool. Keeping these in one place
//! prevents per-tool drift — every tool that accepts `timeout_ms` should call
//! [`clamp_timeout`], every tool that builds a response should go through
//! [`ok_result`] / [`err_result`].

use std::time::Duration;

use rmcp::model::{CallToolResult, Content};
use serde_json::Value;

use crate::errors::{ErrorCode, ErrorResponse, RetryHint};
use crate::response::ToolResponse;
use crate::session::BrowserSession;

/// Upper bound for per-call `timeout_ms` — anything larger gets clamped. Keeps
/// a rogue client from pinning a CDP session for hours on a typoed value. When
/// the clamp fires, a `warnings` entry is added to the response so the caller
/// knows the effective value differs from what they asked for.
pub const MAX_TIMEOUT_MS: u64 = 120_000;

/// Upper bound for `tree` output size — the AX tree of a big page can blow up
/// LLM context if not capped. Clamped silently to the cap; callers see the
/// effective truncation through `data.node_count > tree line count`.
pub const MAX_TREE_NODES: u32 = 5_000;

/// Maximum byte length of a `goto` URL. RFC doesn't mandate a hard limit but
/// 2048 covers every practical real-world URL and blocks megabyte-sized
/// prompt-injection payloads that would otherwise burn CPU in `url::Url::parse`
/// and flood logs.
pub const MAX_URL_LEN: usize = 2048;

/// URL schemes accepted by `goto`. Everything else — including but not limited
/// to `file://`, `data:`, `javascript:`, `chrome:`, `about:`, `blob:`,
/// `view-source:`, `intent:`, `filesystem:`, `chrome-extension:`, `ws://`,
/// `ftp://` — is rejected with `InvalidArgs` so a prompt-injection payload
/// can't pivot the browser onto the local filesystem, an app-launch URI, or
/// an in-page code-execution protocol. The allowlist is explicit (not
/// blacklist) per OWASP fail-closed guidance.
pub const ALLOWED_GOTO_SCHEMES: &[&str] = &["http", "https"];

/// Cap on the UTF-16 code-unit length of `text` tool output. The tree of a
/// big page can blow up LLM context; this cap matches MCP-friendly response
/// sizes. Counted in JS `.length` units (UTF-16), not bytes — surrogate-pair
/// emoji each cost 2 units. Truncation is applied page-side in `Runtime.evaluate`
/// so we never even shuttle the oversized payload over CDP.
pub const MAX_PAGE_TEXT_LEN: usize = 50_000;

/// Cap on the input length of the `type` tool. 4 KiB is well above any
/// legitimate keystroke sequence; anything larger is almost certainly a
/// prompt-injection payload or a buggy caller. Rejected with `InvalidArgs`.
pub const MAX_TYPE_TEXT_LEN: usize = 4_096;

/// Cap on the UTF-16 code-unit length of `html` tool output. Higher than
/// `MAX_PAGE_TEXT_LEN` because rendered HTML carries markup overhead — a
/// page that produces 50 KB of text often produces 200+ KB of HTML.
/// Truncation applied page-side, same rationale as `MAX_PAGE_TEXT_LEN`.
pub const MAX_HTML_LEN: usize = 200_000;

/// Resolve a `@e<N>` ref to a DOM `backendNodeId`. Returns the id on success,
/// or a ready-to-emit error response on failure:
///
/// - In the map but mapped to `None` → `ELEMENT_NOT_FOUND` (the AX node
///   exists but has no DOM counterpart, e.g. a text fragment or a virtual
///   scrollable group; clicking it is meaningless).
/// - Not in the map, but N ≤ max_ref ever issued → `NODE_STALE` (the ref
///   was valid in a prior snapshot, the page has since navigated/refreshed).
/// - Not in the map, and N > max_ref or unparseable → `NODE_UNKNOWN` (no
///   snapshot ever produced this ref; almost certainly a hallucination or
///   typo). Different recovery: the caller can't just re-`tree`, the LLM
///   needs to actually look at the snapshot output and pick a real ref.
pub(crate) async fn resolve_ref(
    session: &BrowserSession,
    ref_id: &str,
) -> Result<i64, ErrorResponse> {
    match session.lookup_ref(ref_id).await {
        Ok(Some(id)) => Ok(id),
        Ok(None) => Err(ErrorResponse::new(
            ErrorCode::ElementNotFound,
            format!("ref {ref_id} resolves to an AX node with no DOM mapping"),
        )),
        Err(()) => {
            let max = session.max_ref();
            let parsed = crate::session::parse_ref_index(ref_id);
            let is_known_range = parsed.is_some_and(|n| n >= 1 && n <= max);
            if is_known_range {
                Err(ErrorResponse::new(
                    ErrorCode::NodeStale,
                    format!(
                        "ref {ref_id} is from an older snapshot (the ref map \
                         was replaced by a later `tree` call or cleared on \
                         navigation) — call `tree` again to get fresh refs"
                    ),
                )
                .with_retry(RetryHint::Snapshot))
            } else {
                // Hint depends on cause: only "no snapshot yet" benefits from
                // a `tree` retry. A ref that exceeds the issued max OR a
                // malformed `@eN` is a typo / hallucination — re-snapshotting
                // won't surface it. Returning `RetryHint::None` for those
                // signals "fix your ref" instead of looping forever.
                let (detail, hint) = match parsed {
                    Some(n) if max == 0 => (
                        format!(
                            "ref {ref_id} requested but no `tree` snapshot has been taken yet (n={n})"
                        ),
                        RetryHint::Snapshot,
                    ),
                    Some(n) => (
                        format!(
                            "ref {ref_id} (n={n}) exceeds the highest ref ever issued in this session (max={max})"
                        ),
                        RetryHint::None,
                    ),
                    None => (
                        format!("ref {ref_id:?} is not a valid `@e<N>` ref"),
                        RetryHint::None,
                    ),
                };
                Err(ErrorResponse::new(ErrorCode::NodeUnknown, detail).with_retry(hint))
            }
        }
    }
}

pub(crate) fn ok_result<T: serde::Serialize>(resp: &ToolResponse<T>) -> CallToolResult {
    CallToolResult::success(vec![Content::text(resp.to_json())])
}

pub(crate) fn err_result(err: &ErrorResponse) -> CallToolResult {
    let mut result = CallToolResult::success(vec![Content::text(err.to_json())]);
    result.is_error = Some(true);
    result
}

/// Apply [`MAX_TIMEOUT_MS`] to a caller-supplied `timeout_ms` and return both
/// the effective `Duration` and a flag indicating whether clamping occurred.
/// Pure — unit-testable without a CDP connection.
pub(crate) fn clamp_timeout(timeout_ms: Option<u64>, default: Duration) -> (Duration, bool) {
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

/// Apply [`MAX_TREE_NODES`] to a caller-supplied `max_nodes`, defaulting to
/// [`DEFAULT_TREE_NODES`] when unset. Returns both the effective value and
/// a clamp flag. Pure.
pub(crate) fn clamp_max_nodes(max_nodes: Option<u32>) -> (u32, bool) {
    match max_nodes {
        Some(n) => {
            let clamped = n > MAX_TREE_NODES;
            (n.min(MAX_TREE_NODES), clamped)
        }
        None => (DEFAULT_TREE_NODES, false),
    }
}

/// Default `max_nodes` when the caller doesn't specify. Bumped from 500 to
/// 1500 in v0.4.1 after R3 dogfood: modern docs SPAs (react.dev, MDN) put the
/// nav sidebar past index 500, so the default-clamped tree often missed the
/// link the LLM was looking for. 1500 covers the sidebar-plus-content slice
/// of every site we measured while staying well under the 5000 hard cap.
pub(crate) const DEFAULT_TREE_NODES: u32 = 1_500;

/// Emit a `SessionClosed` error with `RetryHint::NewSession` when no session
/// has been opened yet. The retry hint tells the LLM to call `goto` (which
/// auto-creates the default session) rather than ping `tree`/`text` again
/// hoping the *element* will appear.
pub(crate) fn no_session_err() -> ErrorResponse {
    ErrorResponse::new(
        ErrorCode::SessionClosed,
        "no session yet — call `goto` first",
    )
    .with_retry(RetryHint::NewSession)
}

/// Same shape as [`no_session_err`] but for the case where the session
/// exists but `ensure_attached` hasn't run yet (no CDP target id). In
/// practice this only happens if a tool is called between session creation
/// and the first `goto` — the retry hint is the same: open a new session
/// (or just call `goto`, which attaches lazily).
pub(crate) fn no_target_err() -> ErrorResponse {
    ErrorResponse::new(
        ErrorCode::SessionClosed,
        "session has no attached target — call `goto` first",
    )
    .with_retry(RetryHint::NewSession)
}

/// Validate the `selector`/`ref` pair carried by every targeted tool
/// (`click`, `fill`, etc.). Returns an `ErrorResponse` to surface and short-
/// circuit on; `None` means the inputs are well-formed.
///
/// Two failure modes:
/// 1. Both unset, or both set → `NoSelector` (recovery: pick exactly one).
/// 2. `selector` set but empty string → `InvalidArgs`
///    (recovery: send a real CSS selector or use `ref`).
///
/// Pulled out of each tool's `handle()` so the contract is stated once and
/// can be unit-tested without spinning up a session.
pub(crate) fn validate_selector_or_ref(
    selector: Option<&str>,
    ref_id: Option<&str>,
) -> Option<ErrorResponse> {
    if selector.is_some() == ref_id.is_some() {
        return Some(ErrorResponse::new(
            ErrorCode::NoSelector,
            "exactly one of `selector` or `ref` is required",
        ));
    }
    if let Some(s) = selector
        && s.is_empty()
    {
        return Some(ErrorResponse::new(
            ErrorCode::InvalidArgs,
            "selector must not be empty",
        ));
    }
    if let Some(r) = ref_id
        && r.is_empty()
    {
        return Some(ErrorResponse::new(
            ErrorCode::InvalidArgs,
            "ref must not be empty",
        ));
    }
    None
}

/// Outcome of a `Runtime.evaluate` call. Distinguishes "the JS threw" from
/// "the CDP transport failed" — they map to different error codes
/// (`InvalidExpression` vs `CdpError`) and the LLM should react differently.
pub(crate) enum EvalOutcome {
    /// Expression returned cleanly. `value` is the `result.value` field
    /// (`returnByValue=true`); `description` is `result.description` for
    /// non-primitive returns where `value` is absent.
    Ok {
        value: Option<Value>,
        description: Option<String>,
    },
    /// Expression itself threw. Carries the human-readable exception message.
    Threw(String),
}

/// Map a `DOM.resolveNode` transport error string into the right structured
/// response. Chromium phrases "the backend node id is no longer attached to a
/// document" several different ways depending on whether the document was
/// swapped, the node was detached mid-call, or the id was never valid. All of
/// those collapse into `NODE_STALE` for the caller — re-snapshot is the
/// correct recovery in every case. Anything else stays `CDP_ERROR` so a real
/// transport bug doesn't masquerade as user error.
fn map_resolve_node_error(ref_id: &str, error_msg: &str) -> ErrorResponse {
    let lower = error_msg.to_ascii_lowercase();
    let is_stale = lower.contains("does not belong to the document")
        || lower.contains("could not find node")
        || lower.contains("no node with given id")
        || lower.contains("node with given id");
    if is_stale {
        ErrorResponse::new(
            ErrorCode::NodeStale,
            format!("ref {ref_id} no longer attached to the current document — call `tree` again"),
        )
        .with_retry(RetryHint::Snapshot)
    } else {
        ErrorResponse::new(
            ErrorCode::CdpError,
            format!("DOM.resolveNode failed: {error_msg}"),
        )
    }
}

/// Resolve a `@e<N>` ref into a `Runtime.RemoteObject` `objectId` — the
/// handle CDP needs to call methods on the element via
/// `Runtime.callFunctionOn`. Two-step: ref → backendNodeId → DOM.resolveNode.
pub(crate) async fn ref_to_object_id(
    session: &BrowserSession,
    cdp_sid: &str,
    ref_id: &str,
    timeout: Duration,
) -> Result<String, ErrorResponse> {
    let backend_id = resolve_ref(session, ref_id).await?;
    let resp = session
        .conn
        .send_recv(
            "DOM.resolveNode",
            serde_json::json!({ "backendNodeId": backend_id }),
            Some(cdp_sid),
            timeout,
        )
        .await
        .map_err(|e| map_resolve_node_error(ref_id, &e.to_string()))?;
    let object_id = resp
        .get("object")
        .and_then(|o| o.get("objectId"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ErrorResponse::new(
                ErrorCode::ElementNotFound,
                format!("ref {ref_id} backend node has no resolvable RemoteObject"),
            )
        })?;
    Ok(object_id.to_string())
}

/// Call a JavaScript function on a previously resolved `objectId` (the `this`
/// of the function body). `function_declaration` must be a complete function
/// expression, e.g. `"function(v) { this.value = v; }"`. `arguments_json` is
/// the literal JSON array passed as `arguments`.
pub(crate) async fn call_function_on(
    session: &BrowserSession,
    cdp_sid: &str,
    object_id: &str,
    function_declaration: &str,
    arguments: serde_json::Value,
    timeout: Duration,
) -> Result<EvalOutcome, ErrorResponse> {
    let resp = session
        .conn
        .send_recv(
            "Runtime.callFunctionOn",
            serde_json::json!({
                "objectId": object_id,
                "functionDeclaration": function_declaration,
                "arguments": arguments,
                "returnByValue": true,
                "awaitPromise": true,
            }),
            Some(cdp_sid),
            timeout,
        )
        .await
        .map_err(|e| {
            ErrorResponse::new(
                ErrorCode::CdpError,
                format!("Runtime.callFunctionOn failed: {e}"),
            )
        })?;

    if let Some(exc) = resp.get("exceptionDetails") {
        let msg = exc
            .get("exception")
            .and_then(|v| v.get("description").or_else(|| v.get("value")))
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| exc.get("text").and_then(|v| v.as_str()).map(String::from))
            .unwrap_or_else(|| "callFunctionOn threw".to_string());
        return Ok(EvalOutcome::Threw(msg));
    }
    let result = resp.get("result");
    let value = result.and_then(|r| r.get("value")).cloned();
    let description = result
        .and_then(|r| r.get("description"))
        .and_then(|v| v.as_str())
        .map(String::from);
    Ok(EvalOutcome::Ok { value, description })
}

/// Release a `Runtime.RemoteObject` `objectId` previously obtained via
/// [`ref_to_object_id`]. Call this in tool cleanup paths so the page-side
/// object table doesn't accumulate stale handles for the duration of the
/// session. Errors are deliberately swallowed: by the time we're releasing
/// the handle the tool's success/error has already been determined, and a
/// failed release shouldn't change what the LLM sees. Logged at `debug`
/// level for forensics.
pub(crate) async fn release_object_id(
    session: &BrowserSession,
    cdp_sid: &str,
    object_id: &str,
    timeout: Duration,
) {
    let res = session
        .conn
        .send_recv(
            "Runtime.releaseObject",
            serde_json::json!({ "objectId": object_id }),
            Some(cdp_sid),
            timeout,
        )
        .await;
    if let Err(e) = res {
        tracing::debug!(error = %e, "Runtime.releaseObject failed (non-fatal)");
    }
}

/// Run `Runtime.evaluate` against the session's current target with
/// `returnByValue=true` and `awaitPromise=true`. Centralises the JSON
/// boilerplate so every tool that needs to poke the page (text, html,
/// evaluate, fill, storage…) shares one CDP shape.
pub(crate) async fn runtime_evaluate(
    session: &BrowserSession,
    cdp_sid: &str,
    expression: &str,
    timeout: Duration,
) -> Result<EvalOutcome, ErrorResponse> {
    let resp = session
        .conn
        .send_recv(
            "Runtime.evaluate",
            serde_json::json!({
                "expression": expression,
                "returnByValue": true,
                "awaitPromise": true,
            }),
            Some(cdp_sid),
            timeout,
        )
        .await
        .map_err(|e| {
            ErrorResponse::new(ErrorCode::CdpError, format!("Runtime.evaluate failed: {e}"))
        })?;

    if let Some(exc) = resp.get("exceptionDetails") {
        let msg = exc
            .get("exception")
            .and_then(|v| v.get("description").or_else(|| v.get("value")))
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| exc.get("text").and_then(|v| v.as_str()).map(String::from))
            .unwrap_or_else(|| "expression threw".to_string());
        return Ok(EvalOutcome::Threw(msg));
    }

    let result = resp.get("result");
    let value = result.and_then(|r| r.get("value")).cloned();
    let description = result
        .and_then(|r| r.get("description"))
        .and_then(|v| v.as_str())
        .map(String::from);
    Ok(EvalOutcome::Ok { value, description })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(n, DEFAULT_TREE_NODES);
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

    #[test]
    fn resolve_node_error_maps_stale_phrases() {
        for stale in [
            "Node with given id does not belong to the document",
            "Could not find node with given id",
            "No node with given id found",
            "node with given id (123) is gone",
        ] {
            let err = map_resolve_node_error("@e5", stale);
            assert_eq!(
                err.code,
                ErrorCode::NodeStale,
                "expected NODE_STALE for: {stale}"
            );
            assert_eq!(err.retry, Some(RetryHint::Snapshot));
        }
    }

    #[test]
    fn resolve_node_error_passes_through_real_cdp_errors() {
        for real in [
            "WebSocket connection closed unexpectedly",
            "timeout waiting for CDP response",
            "Internal error: out of memory",
        ] {
            let err = map_resolve_node_error("@e5", real);
            assert_eq!(
                err.code,
                ErrorCode::CdpError,
                "expected CDP_ERROR for: {real}"
            );
        }
    }

    #[test]
    fn clamp_timeout_none_floors_oversized_default() {
        let oversized = Duration::from_millis(MAX_TIMEOUT_MS * 10);
        let (d, clamped) = clamp_timeout(None, oversized);
        assert_eq!(d, Duration::from_millis(MAX_TIMEOUT_MS));
        assert!(!clamped);
    }

    #[test]
    fn validate_selector_or_ref_rejects_neither_set() {
        let err = validate_selector_or_ref(None, None).expect("must error");
        assert_eq!(err.code, ErrorCode::NoSelector);
    }

    #[test]
    fn validate_selector_or_ref_rejects_both_set() {
        let err = validate_selector_or_ref(Some("#x"), Some("@e1")).expect("must error");
        assert_eq!(err.code, ErrorCode::NoSelector);
    }

    #[test]
    fn validate_selector_or_ref_rejects_empty_selector() {
        let err = validate_selector_or_ref(Some(""), None).expect("must error");
        assert_eq!(err.code, ErrorCode::InvalidArgs);
        assert!(err.message.contains("must not be empty"));
    }

    #[test]
    fn validate_selector_or_ref_accepts_real_selector() {
        assert!(validate_selector_or_ref(Some("#submit"), None).is_none());
    }

    #[test]
    fn validate_selector_or_ref_accepts_real_ref() {
        assert!(validate_selector_or_ref(None, Some("@e3")).is_none());
    }

    #[test]
    fn validate_selector_or_ref_rejects_empty_ref() {
        // R4 (API contract review) flagged the previous "empty ref slips
        // through to resolve_ref" behavior as asymmetric: empty selector
        // returned `INVALID_ARGS` early, but empty ref returned a delayed
        // `NODE_UNKNOWN` from resolve_ref. We now reject both early so
        // the contract is symmetric.
        let err = validate_selector_or_ref(None, Some("")).expect("must error");
        assert_eq!(err.code, ErrorCode::InvalidArgs);
        assert!(err.message.contains("ref"));
    }
}
