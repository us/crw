//! `network` — drain the session's HTTP request/response ring buffer.
//!
//! The buffer is filled by the listener task spawned in
//! [`crate::session::BrowserSession::ensure_attached`]; this tool only
//! snapshots its current contents.

use std::time::Instant;

use rmcp::{ErrorData as McpError, model::CallToolResult, schemars};
use serde::{Deserialize, Serialize};

use crate::errors::{ErrorCode, ErrorResponse};
use crate::response::ToolResponse;
use crate::server::CrwBrowse;
use crate::session::NetworkEntry;
use crate::tools::common::{err_result, no_session_err, ok_result};

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct NetworkInput {
    /// Filter mode: `all` (default), `failed` (status >= 400), `requests`
    /// (only `Network.requestWillBeSent` entries), `responses` (only
    /// `Network.responseReceived` entries). Singular `request` / `response`
    /// are accepted as aliases. Case-insensitive.
    #[serde(default)]
    pub filter: Option<String>,
    /// When `true`, drain the buffer (clear after snapshot). Default `false`.
    #[serde(default)]
    pub clear: bool,
}

#[derive(Debug, Serialize)]
pub struct NetworkData {
    pub count: usize,
    pub entries: Vec<NetworkEntry>,
}

pub async fn handle(server: &CrwBrowse, input: NetworkInput) -> Result<CallToolResult, McpError> {
    let started = Instant::now();

    // Validate `filter` BEFORE draining so a typo on a `clear: true` call
    // doesn't silently empty the buffer before erroring out.
    // Postel: case-insensitive (mirrors `console.rs`) + accept the
    // singular forms ("request", "response") as aliases for the
    // documented plural since callers reasonably expect both.
    let filter_lc = input.filter.as_deref().map(str::to_lowercase);
    let mode = match filter_lc.as_deref() {
        None | Some("all") => "all",
        Some("requests") | Some("request") => "requests",
        Some("responses") | Some("response") => "responses",
        Some("failed") => "failed",
        Some(_) => {
            // Echo the original spelling so the user sees what they
            // sent, not its lowercased shadow.
            let other = input.filter.as_deref().unwrap_or("");
            return Ok(err_result(&ErrorResponse::new(
                ErrorCode::InvalidArgs,
                format!("unknown filter {other:?} — expected one of all|failed|requests|responses"),
            )));
        }
    };

    let Some(session) = server.default_session_get().await else {
        return Ok(err_result(&no_session_err()));
    };

    let entries = session.network_drain(input.clear).await;
    let entries: Vec<NetworkEntry> = match mode {
        "all" => entries,
        "requests" => entries
            .into_iter()
            .filter(|e| e.kind == "request")
            .collect(),
        "responses" => entries
            .into_iter()
            .filter(|e| e.kind == "response")
            .collect(),
        "failed" => entries
            .into_iter()
            .filter(|e| e.status.is_some_and(|s| s >= 400))
            .collect(),
        _ => unreachable!("validated above"),
    };
    let count = entries.len();

    let payload = ToolResponse::new(
        &session.short_id,
        session.last_url().await,
        NetworkData { count, entries },
    )
    .with_elapsed_ms(started.elapsed().as_millis() as u64);
    Ok(ok_result(&payload))
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
    async fn network_rejects_unknown_filter() {
        let server = CrwBrowse::new(BrowseConfig::default());
        let res = handle(
            &server,
            NetworkInput {
                filter: Some("requesting".into()),
                clear: false,
            },
        )
        .await
        .expect("handle");
        let body = err_text(&res);
        assert!(
            body.contains("unknown filter") && body.contains("requesting"),
            "expected unknown-filter rejection, got: {body}"
        );
    }

    #[tokio::test]
    async fn network_clear_does_not_drain_when_filter_invalid() {
        // Regression: validating before drain means a bad filter on
        // `clear: true` doesn't silently empty the buffer.
        let server = CrwBrowse::new(BrowseConfig::default());
        let res = handle(
            &server,
            NetworkInput {
                filter: Some("nope".into()),
                clear: true,
            },
        )
        .await
        .expect("handle");
        let body = err_text(&res);
        assert!(body.contains("unknown filter"), "got: {body}");
    }
}
