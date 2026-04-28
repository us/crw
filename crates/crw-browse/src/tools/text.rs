//! `text` — extract visible text from the page (or a CSS-selected subtree).
//!
//! Implementation: a single `Runtime.evaluate` that resolves the selector and
//! reads `innerText`. We intentionally use `innerText` (not `textContent`) so
//! the result reflects what a human would see — hidden elements are dropped
//! and whitespace collapses the way the browser renders it.

use std::time::Instant;

use rmcp::{ErrorData as McpError, model::CallToolResult, schemars};
use serde::{Deserialize, Serialize};

use crate::errors::{ErrorCode, ErrorResponse};
use crate::response::ToolResponse;
use crate::server::CrwBrowse;
use crate::tools::common::{
    EvalOutcome, MAX_PAGE_TEXT_LEN, MAX_TIMEOUT_MS, clamp_timeout, err_result, no_session_err,
    no_target_err, ok_result, runtime_evaluate,
};

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct TextInput {
    /// Optional CSS selector. When omitted, the entire `document.body` is
    /// read. When given, only the matched element's `innerText` is returned.
    #[serde(default)]
    pub selector: Option<String>,
    /// Read timeout in milliseconds (default: 30000, capped at 120000).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct TextData {
    pub text: String,
    /// `true` when the page-side script trimmed the result to fit
    /// [`MAX_PAGE_TEXT_LEN`] (UTF-16 code-units; surrogate-pair emoji each
    /// cost 2 units). Agents can re-call with a narrower selector.
    pub truncated: bool,
}

pub async fn handle(server: &CrwBrowse, input: TextInput) -> Result<CallToolResult, McpError> {
    let started = Instant::now();
    let (timeout, timeout_clamped) = clamp_timeout(input.timeout_ms, server.config().page_timeout);

    let Some(session) = server.default_session_get().await else {
        return Ok(err_result(&no_session_err()));
    };
    let Some(cdp_sid) = session.cdp_session_id().await else {
        return Ok(err_result(&no_target_err()));
    };

    // Build the page-side script. The script returns `{found, text, truncated}`
    // so we can distinguish "selector matched nothing" from "selector matched
    // and text was empty".
    let expression = match input.selector.as_deref() {
        None => format!(
            r#"(() => {{
                const t = document.body ? document.body.innerText : "";
                const cap = {cap};
                if (t.length > cap) {{
                    return {{ found: true, text: t.slice(0, cap) + "…", truncated: true }};
                }}
                return {{ found: true, text: t, truncated: false }};
            }})()"#,
            cap = MAX_PAGE_TEXT_LEN
        ),
        Some(sel) => {
            let sel_json = serde_json::to_string(sel).unwrap_or_else(|_| "\"\"".into());
            format!(
                r#"(() => {{
                    const el = document.querySelector({sel});
                    if (!el) return {{ found: false }};
                    const t = el.innerText || "";
                    const cap = {cap};
                    if (t.length > cap) {{
                        return {{ found: true, text: t.slice(0, cap) + "…", truncated: true }};
                    }}
                    return {{ found: true, text: t, truncated: false }};
                }})()"#,
                sel = sel_json,
                cap = MAX_PAGE_TEXT_LEN
            )
        }
    };

    match runtime_evaluate(&session, &cdp_sid, &expression, timeout).await {
        Err(e) => Ok(err_result(&e)),
        Ok(EvalOutcome::Threw(msg)) => Ok(err_result(&ErrorResponse::new(
            ErrorCode::CdpError,
            format!("page-side script threw: {msg}"),
        ))),
        Ok(EvalOutcome::Ok { value, .. }) => {
            let v = value.unwrap_or(serde_json::Value::Null);
            if v.get("found").and_then(|f| f.as_bool()) != Some(true) {
                return Ok(err_result(&ErrorResponse::new(
                    ErrorCode::ElementNotFound,
                    format!(
                        "no element matched selector {:?}",
                        input.selector.as_deref().unwrap_or("")
                    ),
                )));
            }
            let text = v
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            let truncated = v
                .get("truncated")
                .and_then(|t| t.as_bool())
                .unwrap_or(false);

            let mut payload = ToolResponse::new(
                &session.short_id,
                session.last_url().await,
                TextData { text, truncated },
            )
            .with_elapsed_ms(started.elapsed().as_millis() as u64);
            if timeout_clamped {
                payload = payload.with_warning(format!(
                    "timeout_ms clamped to {MAX_TIMEOUT_MS} ms (server-side cap)"
                ));
            }
            Ok(ok_result(&payload))
        }
    }
}
