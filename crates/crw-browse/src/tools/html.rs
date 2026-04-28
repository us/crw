//! `html` — return the `outerHTML` of the page or a CSS-selected element.
//!
//! Uses `Runtime.evaluate` rather than `DOM.getOuterHTML` so we don't have to
//! issue a separate `DOM.querySelector` round-trip for the selector path —
//! the JS one-liner does both in one CDP call. For a session that's been
//! through `tree`, the same shape works; we don't try to resolve `@e<N>`
//! refs here because HTML inspection is most useful BEFORE a snapshot exists.

use std::time::Instant;

use rmcp::{ErrorData as McpError, model::CallToolResult, schemars};
use serde::{Deserialize, Serialize};

use crate::errors::{ErrorCode, ErrorResponse};
use crate::response::ToolResponse;
use crate::server::CrwBrowse;
use crate::tools::common::{
    EvalOutcome, MAX_HTML_LEN, MAX_TIMEOUT_MS, clamp_timeout, err_result, no_session_err,
    no_target_err, ok_result, runtime_evaluate,
};

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct HtmlInput {
    /// Optional CSS selector. When omitted, returns
    /// `document.documentElement.outerHTML` (the full `<html>...</html>`).
    #[serde(default)]
    pub selector: Option<String>,
    /// Read timeout in milliseconds (default: 30000, capped at 120000).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct HtmlData {
    pub html: String,
    /// `true` when the page-side script trimmed the result to fit
    /// [`MAX_HTML_LEN`] (UTF-16 code units, JS `.length`).
    pub truncated: bool,
}

pub async fn handle(server: &CrwBrowse, input: HtmlInput) -> Result<CallToolResult, McpError> {
    let started = Instant::now();
    let (timeout, timeout_clamped) = clamp_timeout(input.timeout_ms, server.config().page_timeout);

    let Some(session) = server.default_session_get().await else {
        return Ok(err_result(&no_session_err()));
    };
    let Some(cdp_sid) = session.cdp_session_id().await else {
        return Ok(err_result(&no_target_err()));
    };

    let expression = match input.selector.as_deref() {
        None => format!(
            r#"(() => {{
                const h = document.documentElement ? document.documentElement.outerHTML : "";
                const cap = {cap};
                if (h.length > cap) return {{ found: true, html: h.slice(0, cap), truncated: true }};
                return {{ found: true, html: h, truncated: false }};
            }})()"#,
            cap = MAX_HTML_LEN
        ),
        Some(sel) => {
            let sel_json = serde_json::to_string(sel).unwrap_or_else(|_| "\"\"".into());
            format!(
                r#"(() => {{
                    const el = document.querySelector({sel});
                    if (!el) return {{ found: false }};
                    const h = el.outerHTML || "";
                    const cap = {cap};
                    if (h.length > cap) return {{ found: true, html: h.slice(0, cap), truncated: true }};
                    return {{ found: true, html: h, truncated: false }};
                }})()"#,
                sel = sel_json,
                cap = MAX_HTML_LEN
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
            let html = v
                .get("html")
                .and_then(|h| h.as_str())
                .unwrap_or("")
                .to_string();
            let truncated = v
                .get("truncated")
                .and_then(|t| t.as_bool())
                .unwrap_or(false);

            let mut payload = ToolResponse::new(
                &session.short_id,
                session.last_url().await,
                HtmlData { html, truncated },
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
