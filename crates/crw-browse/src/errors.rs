//! Structured error taxonomy with retry hints. Serialized into the JSON body
//! of an MCP tool response with `is_error: true` so LLM clients can parse it
//! and choose the right recovery path (retry snapshot, wait, reopen session…).

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    NodeStale,
    Timeout,
    NavBlocked,
    SessionClosed,
    NotFound,
    InvalidArgs,
    BrowserUnavailable,
    CdpError,
    PolicyViolation,
    RateLimited,
    Internal,
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
        }
    }

    pub fn with_retry(mut self, retry: RetryHint) -> Self {
        self.retry = Some(retry);
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
    }

    #[test]
    fn all_codes_roundtrip_to_screaming_snake() {
        let cases = [
            (ErrorCode::NodeStale, "NODE_STALE"),
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
        ];
        for (code, expected) in cases {
            let err = ErrorResponse::new(code, "x");
            let json: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
            assert_eq!(json["code"], expected, "code {code:?}");
        }
    }
}
