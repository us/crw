//! Structured error taxonomy with retry hints. Serialized into the JSON body
//! of an MCP tool response with `is_error: true` so LLM clients can parse it
//! and choose the right recovery path (retry snapshot, wait, reopen session…).

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    NodeStale,
    /// `@e<N>` ref was never produced by any snapshot in this session
    /// (typo, hallucinated ref, or N exceeds last known max). Distinct from
    /// NodeStale, which means the ref *was* valid and has since expired.
    NodeUnknown,
    /// Element exists but is not a sensible interaction target for the
    /// requested action: e.g. `click` on `<html>` / `<body>` / root document,
    /// or `type_text` against a node that is not focusable
    /// (no input/textarea/contenteditable). Caller should pick a different
    /// ref. Despite the historical name, this covers both click and focus
    /// targeting failures — the recovery is identical (resnap and pick
    /// another @e ref).
    NodeNotClickable,
    Timeout,
    NavBlocked,
    SessionClosed,
    NotFound,
    /// Generic input validation failure (empty string where one is required,
    /// out-of-range numeric, malformed JSON shape, mutually-exclusive option
    /// combinations like `wait{ms,selector}`). Tools should prefer this
    /// generic code over inventing per-field codes; reserve specialized codes
    /// for cases that have a distinct recovery action.
    InvalidArgs,
    BrowserUnavailable,
    CdpError,
    PolicyViolation,
    RateLimited,
    Internal,
    /// Caller invoked a target tool (`click`, `fill`) without specifying the
    /// element to act on, or specified both `selector` AND `ref`. This is a
    /// distinct code from `INVALID_ARGS` because the recovery is specific —
    /// the LLM should pick exactly one of the two ways to identify the
    /// element. For other input validation failures, use `INVALID_ARGS`.
    NoSelector,
    ElementNotFound,
    InvalidExpression,
    NotImplemented,
}

/// Hint for how the caller should recover.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetryHint {
    /// Re-fetch the a11y snapshot (stale ref).
    Snapshot,
    /// Back off and retry after the given ms.
    BackoffMs(u64),
    /// Start a new session.
    NewSession,
    /// Permanent — don't retry.
    None,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorResponse {
    pub ok: bool,
    pub code: ErrorCode,
    pub message: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryHint>,

    /// Anchor that was used to resolve a stale ref (populated on NODE_STALE).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_anchor: Option<serde_json::Value>,

    /// Pattern the policy allowed/required (populated on POLICY_VIOLATION).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_pattern: Option<String>,

    /// Tools that ARE allowed (populated on POLICY_VIOLATION).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<Vec<String>>,

    /// Number of items processed before the failure. Tool-specific unit —
    /// `type_text` reports characters typed; future tools that loop over
    /// items (batch click, multi-fill) populate it with their own count.
    /// Lets the caller decide whether the partial state is recoverable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial_count: Option<usize>,
}

impl ErrorResponse {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            code,
            message: message.into(),
            retry: None,
            stale_anchor: None,
            allowed_pattern: None,
            allowed_tools: None,
            partial_count: None,
        }
    }

    pub fn with_retry(mut self, retry: RetryHint) -> Self {
        self.retry = Some(retry);
        self
    }

    pub fn with_partial_count(mut self, count: usize) -> Self {
        self.partial_count = Some(count);
        self
    }

    pub fn node_stale(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NodeStale, message).with_retry(RetryHint::Snapshot)
    }

    pub fn timeout(ms: u64) -> Self {
        Self::new(ErrorCode::Timeout, format!("timed out after {ms} ms"))
            .with_retry(RetryHint::BackoffMs(1000))
    }

    pub fn session_closed() -> Self {
        Self::new(ErrorCode::SessionClosed, "session has been closed")
            .with_retry(RetryHint::NewSession)
    }

    /// Serialize to a plain JSON string for embedding as MCP `text` content.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            r#"{"ok":false,"code":"INTERNAL","message":"serialization failed"}"#.into()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_node_stale_with_retry_snapshot() {
        let err = ErrorResponse::node_stale("node e5 not found");
        let json: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
        assert_eq!(json["ok"], false);
        assert_eq!(json["code"], "NODE_STALE");
        assert_eq!(json["retry"], "snapshot");
    }

    #[test]
    fn serializes_timeout_with_backoff() {
        let err = ErrorResponse::timeout(5000);
        let json: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
        assert_eq!(json["code"], "TIMEOUT");
        assert!(json["message"].as_str().unwrap().contains("5000"));
        assert_eq!(json["retry"]["backoff_ms"], 1000);
    }

    #[test]
    fn skips_none_fields() {
        let err = ErrorResponse::new(ErrorCode::NotFound, "no such element");
        let json = err.to_json();
        assert!(!json.contains("stale_anchor"));
        assert!(!json.contains("allowed_pattern"));
        assert!(!json.contains("retry"));
        assert!(!json.contains("partial_count"));
    }

    #[test]
    fn serializes_partial_count_when_set() {
        let err = ErrorResponse::new(ErrorCode::CdpError, "boom").with_partial_count(3);
        let json: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
        assert_eq!(json["partial_count"], 3);
    }

    #[test]
    fn omits_partial_count_when_none() {
        let err = ErrorResponse::new(ErrorCode::CdpError, "boom");
        let json: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
        assert!(json.get("partial_count").is_none());
    }

    #[test]
    fn all_codes_roundtrip_to_screaming_snake() {
        let cases = [
            (ErrorCode::NodeStale, "NODE_STALE"),
            (ErrorCode::NodeUnknown, "NODE_UNKNOWN"),
            (ErrorCode::NodeNotClickable, "NODE_NOT_CLICKABLE"),
            (ErrorCode::Timeout, "TIMEOUT"),
            (ErrorCode::NavBlocked, "NAV_BLOCKED"),
            (ErrorCode::SessionClosed, "SESSION_CLOSED"),
            (ErrorCode::NotFound, "NOT_FOUND"),
            (ErrorCode::InvalidArgs, "INVALID_ARGS"),
            (ErrorCode::BrowserUnavailable, "BROWSER_UNAVAILABLE"),
            (ErrorCode::CdpError, "CDP_ERROR"),
            (ErrorCode::PolicyViolation, "POLICY_VIOLATION"),
            (ErrorCode::RateLimited, "RATE_LIMITED"),
            (ErrorCode::Internal, "INTERNAL"),
            (ErrorCode::NoSelector, "NO_SELECTOR"),
            (ErrorCode::ElementNotFound, "ELEMENT_NOT_FOUND"),
            (ErrorCode::InvalidExpression, "INVALID_EXPRESSION"),
            (ErrorCode::NotImplemented, "NOT_IMPLEMENTED"),
        ];
        for (code, expected) in cases {
            let err = ErrorResponse::new(code, "x");
            let json: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
            assert_eq!(json["code"], expected, "code {code:?}");
        }
    }
}
