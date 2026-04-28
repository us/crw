//! `console` — drain the session's console-message ring buffer.
//!
//! The buffer is filled by the per-session listener task that
//! [`crate::session::BrowserSession::ensure_attached`] spawns on first attach;
//! this tool just snapshots its current contents (oldest first), optionally
//! filtering by level and/or clearing the buffer in the same call.

use std::time::Instant;

use rmcp::{ErrorData as McpError, model::CallToolResult, schemars};
use serde::{Deserialize, Serialize};

use crate::errors::{ErrorCode, ErrorResponse};
use crate::response::ToolResponse;
use crate::server::CrwBrowse;
use crate::session::ConsoleEntry;
use crate::tools::common::{err_result, no_session_err, ok_result};

/// Console levels that we accept on the input. CDP emits a wider set than
/// vanilla `console.{log,info,...}` — `dir`/`table`/`trace` are real CDP
/// values for the corresponding `console.dir()` etc. — so we accept the full
/// set rather than the narrow user-facing one.
const VALID_LEVELS: &[&str] = &[
    "log", "info", "warn", "warning", "error", "debug", "trace", "dir", "table",
];

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ConsoleInput {
    /// Optional level filter (`error`, `warning`, `log`, `info`, `debug`,
    /// `trace`, `dir`, `table`). Case-insensitive. `warn` is accepted as
    /// an alias for `warning`. When omitted, all levels are returned.
    #[serde(default)]
    pub level: Option<String>,
    /// When `true`, the buffer is cleared after the snapshot is taken so
    /// subsequent calls only see new entries. Default `false`.
    #[serde(default)]
    pub clear: bool,
}

#[derive(Debug, Serialize)]
pub struct ConsoleData {
    pub count: usize,
    pub entries: Vec<ConsoleEntry>,
}

pub async fn handle(server: &CrwBrowse, input: ConsoleInput) -> Result<CallToolResult, McpError> {
    let started = Instant::now();

    // Validate `level` up-front so a typo (e.g. "errors") returns
    // `INVALID_ARGS` with the valid set, instead of silently filtering to
    // zero matches and looking like the page is quiet.
    let needle = match input.level.as_deref() {
        None => None,
        Some(s) => {
            let mut lc = s.to_lowercase();
            // Postel: accept the user-facing "warn" alias and normalize to
            // the CDP-emitted "warning" before filtering. Otherwise a
            // perfectly reasonable `level: "warn"` returns zero rows and
            // looks like the page is quiet.
            if lc == "warn" {
                lc = "warning".to_string();
            }
            if !VALID_LEVELS.contains(&lc.as_str()) {
                return Ok(err_result(&ErrorResponse::new(
                    ErrorCode::InvalidArgs,
                    format!("unknown level '{s}' — expected one of {:?}", VALID_LEVELS),
                )));
            }
            Some(lc)
        }
    };

    let Some(session) = server.default_session_get().await else {
        return Ok(err_result(&no_session_err()));
    };

    let entries = session.console_drain(input.clear).await;
    let entries = match needle {
        Some(needle) => entries
            .into_iter()
            .filter(|e| e.level.to_lowercase() == needle)
            .collect::<Vec<_>>(),
        None => entries,
    };
    let count = entries.len();

    let payload = ToolResponse::new(
        &session.short_id,
        session.last_url().await,
        ConsoleData { count, entries },
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
    async fn console_rejects_unknown_level() {
        let server = CrwBrowse::new(BrowseConfig::default());
        let res = handle(
            &server,
            ConsoleInput {
                level: Some("errors".into()),
                clear: false,
            },
        )
        .await
        .expect("handle");
        let body = err_text(&res);
        assert!(
            body.contains("unknown level") && body.contains("errors"),
            "expected unknown-level rejection, got: {body}"
        );
    }

    #[tokio::test]
    async fn console_accepts_known_level_case_insensitive() {
        // Validation passes; downstream returns SESSION_CLOSED because no
        // session exists. We're only checking validation didn't reject.
        let server = CrwBrowse::new(BrowseConfig::default());
        let res = handle(
            &server,
            ConsoleInput {
                level: Some("WARNING".into()),
                clear: false,
            },
        )
        .await
        .expect("handle");
        // Has to error (no session), but NOT for an unknown-level reason.
        let body = err_text(&res);
        assert!(
            !body.contains("unknown level"),
            "WARNING should be accepted, got: {body}"
        );
    }
}
