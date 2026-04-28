//! `storage` — read or write cookies / localStorage / sessionStorage.
//!
//! Cookies use `Network.getCookies` / `Network.setCookies` directly. The
//! browser-side Storage twins (`localStorage`, `sessionStorage`) ride over
//! `Runtime.evaluate` because Lightpanda lacks a `DOMStorage` domain
//! implementation and Runtime is the lowest-common-denominator path.

use std::time::{Duration, Instant};

use rmcp::{ErrorData as McpError, model::CallToolResult, schemars};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::{ErrorCode, ErrorResponse};
use crate::response::ToolResponse;
use crate::server::CrwBrowse;
use crate::tools::common::{
    EvalOutcome, MAX_TIMEOUT_MS, clamp_timeout, err_result, no_session_err, no_target_err,
    ok_result, runtime_evaluate,
};

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct StorageInput {
    /// `get` (read all), `set` (write one key), `clear` (wipe all entries
    /// of the chosen kind for the current origin).
    pub action: String,
    /// `cookie`, `local` (localStorage), or `session` (sessionStorage).
    pub kind: String,
    /// Required for `set`. For cookies, this is the cookie name.
    #[serde(default)]
    pub key: Option<String>,
    /// Required for `set`. JSON-stringified for storage; passed verbatim to
    /// `Network.setCookies` for cookies.
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct StorageData {
    /// Action that ran (echoed for clarity).
    pub action: String,
    pub kind: String,
    /// Populated for `get`. Cookies: array of CDP cookie objects. local /
    /// session: object map of `{ key: value }` strings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

pub async fn handle(server: &CrwBrowse, input: StorageInput) -> Result<CallToolResult, McpError> {
    let started = Instant::now();
    let (timeout, timeout_clamped) = clamp_timeout(input.timeout_ms, server.config().page_timeout);

    let Some(session) = server.default_session_get().await else {
        return Ok(err_result(&no_session_err()));
    };
    let Some(cdp_sid) = session.cdp_session_id().await else {
        return Ok(err_result(&no_target_err()));
    };

    let action = input.action.to_lowercase();
    let kind = input.kind.to_lowercase();

    let mut extra_warnings: Vec<String> = Vec::new();
    let data_result: Result<Option<Value>, ErrorResponse> = match (action.as_str(), kind.as_str()) {
        ("get", "cookie") => get_cookies(&session, &cdp_sid, timeout).await,
        ("set", "cookie") => set_cookie(&session, &cdp_sid, &input, timeout)
            .await
            .map(|_| None),
        ("clear", "cookie") => match clear_cookies(&session, &cdp_sid, timeout).await {
            Ok(ws) => {
                extra_warnings.extend(ws);
                Ok(None)
            }
            Err(e) => Err(e),
        },
        ("get", "local") | ("get", "session") => {
            get_storage(&session, &cdp_sid, &kind, timeout).await
        }
        ("set", "local") | ("set", "session") => {
            set_storage(&session, &cdp_sid, &kind, &input, timeout)
                .await
                .map(|_| None)
        }
        ("clear", "local") | ("clear", "session") => {
            clear_storage(&session, &cdp_sid, &kind, timeout)
                .await
                .map(|_| None)
        }
        (a, k) => Err(ErrorResponse::new(
            ErrorCode::InvalidArgs,
            format!(
                "unsupported (action,kind) = ({a:?},{k:?}) — expected action in get|set|clear and kind in cookie|local|session"
            ),
        )),
    };

    match data_result {
        Err(e) => Ok(err_result(&e)),
        Ok(data) => {
            let mut payload = ToolResponse::new(
                &session.short_id,
                session.last_url().await,
                StorageData { action, kind, data },
            )
            .with_elapsed_ms(started.elapsed().as_millis() as u64);
            if timeout_clamped {
                payload = payload.with_warning(format!(
                    "timeout_ms clamped to {MAX_TIMEOUT_MS} ms (server-side cap)"
                ));
            }
            for w in extra_warnings {
                payload = payload.with_warning(w);
            }
            Ok(ok_result(&payload))
        }
    }
}

async fn get_cookies(
    session: &crate::session::BrowserSession,
    cdp_sid: &str,
    timeout: Duration,
) -> Result<Option<Value>, ErrorResponse> {
    let resp = session
        .conn
        .send_recv(
            "Network.getCookies",
            serde_json::json!({}),
            Some(cdp_sid),
            timeout,
        )
        .await
        .map_err(|e| {
            ErrorResponse::new(
                ErrorCode::CdpError,
                format!("Network.getCookies failed: {e}"),
            )
        })?;
    Ok(Some(
        resp.get("cookies").cloned().unwrap_or(Value::Array(vec![])),
    ))
}

async fn set_cookie(
    session: &crate::session::BrowserSession,
    cdp_sid: &str,
    input: &StorageInput,
    timeout: Duration,
) -> Result<(), ErrorResponse> {
    let name = input
        .key
        .as_deref()
        .ok_or_else(|| ErrorResponse::new(ErrorCode::InvalidArgs, "set cookie requires `key`"))?;
    let value = input
        .value
        .as_deref()
        .ok_or_else(|| ErrorResponse::new(ErrorCode::InvalidArgs, "set cookie requires `value`"))?;
    let url = session
        .last_url()
        .await
        .ok_or_else(|| ErrorResponse::new(ErrorCode::NotFound, "no url — call `goto` first"))?;
    session
        .conn
        .send_recv(
            "Network.setCookie",
            serde_json::json!({ "name": name, "value": value, "url": url }),
            Some(cdp_sid),
            timeout,
        )
        .await
        .map_err(|e| {
            ErrorResponse::new(
                ErrorCode::CdpError,
                format!("Network.setCookie failed: {e}"),
            )
        })?;
    Ok(())
}

/// Origin-scoped cookie clear. We deliberately do NOT use
/// `Network.clearBrowserCookies`, which wipes the entire profile and would
/// nuke unrelated session state in long-lived browsers shared by other
/// concurrent agents. Instead: read the cookies visible to the current
/// origin, then delete each one by `(name, url)` so only this origin's
/// cookies are affected. Caveats:
/// - Cookies set via `Set-Cookie` with broader scope (e.g. parent domain)
///   that *also* apply to the current origin are deleted here, which is the
///   right behaviour — the agent asked for "clear cookies for this site".
/// - Cookies for a *different* origin are untouched, even if the same browser
///   profile holds them.
async fn clear_cookies(
    session: &crate::session::BrowserSession,
    cdp_sid: &str,
    timeout: Duration,
) -> Result<Vec<String>, ErrorResponse> {
    let url = session
        .last_url()
        .await
        .ok_or_else(|| ErrorResponse::new(ErrorCode::NotFound, "no url — call `goto` first"))?;
    let resp = session
        .conn
        .send_recv(
            "Network.getCookies",
            serde_json::json!({ "urls": [url.clone()] }),
            Some(cdp_sid),
            timeout,
        )
        .await
        .map_err(|e| {
            ErrorResponse::new(
                ErrorCode::CdpError,
                format!("Network.getCookies failed: {e}"),
            )
        })?;
    let cookies = resp
        .get("cookies")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut warnings: Vec<String> = Vec::new();
    let mut failed_names: Vec<String> = Vec::new();
    for cookie in cookies {
        let Some(name) = cookie.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        // Empty cookie names round-trip as a no-op against Chrome but
        // still cost a CDP call. Skip locally so we neither waste the
        // call nor fold a phantom failure into the warning list.
        if name.is_empty() {
            continue;
        }
        // Pass `domain` and `path` from the original cookie when present.
        // Without them, `Network.deleteCookies` derives those from `url`,
        // which fails to match host-only vs domain cookies (e.g. a
        // `.example.com` cookie won't be cleared by deleting against
        // `https://sub.example.com/`). Passing both lets CDP disambiguate.
        let mut params = serde_json::json!({ "name": name, "url": url });
        if let Some(domain) = cookie.get("domain").and_then(|v| v.as_str()) {
            params["domain"] = serde_json::Value::String(domain.to_string());
        }
        if let Some(path) = cookie.get("path").and_then(|v| v.as_str()) {
            params["path"] = serde_json::Value::String(path.to_string());
        }
        if let Err(e) = session
            .conn
            .send_recv("Network.deleteCookies", params, Some(cdp_sid), timeout)
            .await
        {
            failed_names.push(name.to_string());
            tracing::warn!(cookie_name = name, error = %e, "Network.deleteCookies failed");
        }
    }
    if !failed_names.is_empty() {
        // Surface the actual cookie names so the MCP client can decide
        // what to do — the previous "see logs" phrasing pointed the LLM
        // at a `tracing::warn!` stream it can't reach.
        warnings.push(format!(
            "failed to delete {} cookie(s): {:?} (rest cleared)",
            failed_names.len(),
            failed_names
        ));
    }
    Ok(warnings)
}

async fn get_storage(
    session: &crate::session::BrowserSession,
    cdp_sid: &str,
    kind: &str,
    timeout: Duration,
) -> Result<Option<Value>, ErrorResponse> {
    let store = if kind == "local" {
        "localStorage"
    } else {
        "sessionStorage"
    };
    let expr = format!(
        r#"(() => {{
            const out = {{}};
            for (let i = 0; i < {store}.length; i++) {{
                const k = {store}.key(i);
                if (k !== null) out[k] = {store}.getItem(k);
            }}
            return out;
        }})()"#
    );
    match runtime_evaluate(session, cdp_sid, &expr, timeout).await? {
        EvalOutcome::Threw(msg) => Err(ErrorResponse::new(
            ErrorCode::CdpError,
            format!("storage read threw: {msg}"),
        )),
        EvalOutcome::Ok { value, .. } => {
            Ok(Some(value.unwrap_or(Value::Object(Default::default()))))
        }
    }
}

async fn set_storage(
    session: &crate::session::BrowserSession,
    cdp_sid: &str,
    kind: &str,
    input: &StorageInput,
    timeout: Duration,
) -> Result<(), ErrorResponse> {
    let key = input
        .key
        .as_deref()
        .ok_or_else(|| ErrorResponse::new(ErrorCode::InvalidArgs, "set requires `key`"))?;
    let value = input
        .value
        .as_deref()
        .ok_or_else(|| ErrorResponse::new(ErrorCode::InvalidArgs, "set requires `value`"))?;
    let store = if kind == "local" {
        "localStorage"
    } else {
        "sessionStorage"
    };
    let key_json = serde_json::to_string(&key).unwrap_or_else(|_| "\"\"".into());
    let val_json = serde_json::to_string(&value).unwrap_or_else(|_| "\"\"".into());
    let expr = format!("{store}.setItem({key_json}, {val_json}); true");
    match runtime_evaluate(session, cdp_sid, &expr, timeout).await? {
        EvalOutcome::Threw(msg) => Err(ErrorResponse::new(
            ErrorCode::CdpError,
            format!("storage write threw: {msg}"),
        )),
        EvalOutcome::Ok { .. } => Ok(()),
    }
}

async fn clear_storage(
    session: &crate::session::BrowserSession,
    cdp_sid: &str,
    kind: &str,
    timeout: Duration,
) -> Result<(), ErrorResponse> {
    let store = if kind == "local" {
        "localStorage"
    } else {
        "sessionStorage"
    };
    let expr = format!("{store}.clear(); true");
    match runtime_evaluate(session, cdp_sid, &expr, timeout).await? {
        EvalOutcome::Threw(msg) => Err(ErrorResponse::new(
            ErrorCode::CdpError,
            format!("storage clear threw: {msg}"),
        )),
        EvalOutcome::Ok { .. } => Ok(()),
    }
}
