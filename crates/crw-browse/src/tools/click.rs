//! `click` — click an element, addressed by CSS selector or `@e<N>` ref.
//!
//! Implementation strategy: instead of computing pixel coordinates and
//! dispatching mouse events (which requires `DOM.getBoxModel` + an `Input`
//! domain in working order), we resolve the target to a JS handle and call
//! `.click()` on it. This works identically across Lightpanda and Chrome,
//! triggers the synthetic `click` event the same way a real user click
//! would, and side-steps the coordinate-translation footguns (scroll
//! position, fixed headers, transformed parents).

use std::time::Instant;

use rmcp::{ErrorData as McpError, model::CallToolResult, schemars};
use serde::{Deserialize, Serialize};

use crate::errors::{ErrorCode, ErrorResponse};
use crate::response::ToolResponse;
use crate::server::CrwBrowse;
use crate::tools::common::{
    EvalOutcome, MAX_TIMEOUT_MS, call_function_on, clamp_timeout, err_result, no_session_err,
    no_target_err, ok_result, ref_to_object_id, release_object_id, runtime_evaluate,
    validate_selector_or_ref,
};

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ClickInput {
    /// CSS selector. Mutually exclusive with `ref`. Exactly one must be set.
    #[serde(default)]
    pub selector: Option<String>,
    /// `@e<N>` ref from the most recent `tree` snapshot. Mutually exclusive
    /// with `selector`.
    #[serde(default, rename = "ref")]
    pub ref_id: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct ClickData {
    /// `true` when the click handler completed without throwing. `false`
    /// indicates the element was found but the synthetic click threw on the
    /// page side.
    pub clicked: bool,
}

pub async fn handle(server: &CrwBrowse, input: ClickInput) -> Result<CallToolResult, McpError> {
    let started = Instant::now();
    let (timeout, timeout_clamped) = clamp_timeout(input.timeout_ms, server.config().page_timeout);

    if let Some(err) = validate_selector_or_ref(input.selector.as_deref(), input.ref_id.as_deref())
    {
        return Ok(err_result(&err));
    }

    let Some(session) = server.default_session_get().await else {
        return Ok(err_result(&no_session_err()));
    };
    let Some(cdp_sid) = session.cdp_session_id().await else {
        return Ok(err_result(&no_target_err()));
    };

    let outcome = if let Some(ref_id) = input.ref_id.as_deref() {
        let object_id = match ref_to_object_id(&session, &cdp_sid, ref_id, timeout).await {
            Ok(id) => id,
            Err(e) => return Ok(err_result(&e)),
        };
        // Three things in one round-trip:
        //   1. tagName guard — `<html>` and `<body>` should never be a click
        //      target; clicking them is a no-op (handler runs on document) but
        //      LLMs treat the silent success as "the button worked". Caught
        //      here as NODE_NOT_CLICKABLE so the caller picks a real ref.
        //   2. focus() — Element.click() does NOT focus per WHATWG focusing
        //      steps (https://html.spec.whatwg.org/#focusing-steps), so a
        //      subsequent type_text would fire keys at document.body. Calling
        //      focus() before click() mirrors what a real keyboard-driven
        //      activation does. focus() is a no-op on non-focusable elements,
        //      so it's always safe to call.
        //   3. click() — synthetic click event, same as before.
        let result = call_function_on(
            &session,
            &cdp_sid,
            &object_id,
            "function() { \
                const tag = (this.tagName || '').toLowerCase(); \
                if (this.nodeType !== 1 || tag === 'html' || tag === 'body' || typeof this.click !== 'function') { \
                    return { not_clickable: true, tag: tag || '#document' }; \
                } \
                if (typeof this.focus === 'function') { try { this.focus(); } catch (_) {} } \
                this.click(); \
                return { not_clickable: false }; \
            }",
            serde_json::json!([]),
            timeout,
        )
        .await;
        // Best-effort: free the page-side `RemoteObject` so it doesn't pin
        // memory in CDP's object table for the rest of the session.
        release_object_id(&session, &cdp_sid, &object_id, timeout).await;
        result
    } else {
        // The XOR check above guarantees `selector` is `Some` whenever `ref_id`
        // is `None`; this let-else makes the invariant explicit and avoids a
        // raw `.unwrap()` that would panic if the guard ever drifted.
        let Some(sel) = input.selector.as_deref() else {
            unreachable!("guarded by selector/ref XOR check above")
        };
        let sel_json = serde_json::to_string(sel).unwrap_or_else(|_| "\"\"".into());
        // Same focus + non-clickable check as the ref path; see comment there
        // for the rationale.
        let expr = format!(
            r#"(() => {{
                const el = document.querySelector({sel_json});
                if (!el) return {{ found: false }};
                const tag = (el.tagName || '').toLowerCase();
                if (el.nodeType !== 1 || tag === 'html' || tag === 'body' || typeof el.click !== 'function') {{
                    return {{ found: true, not_clickable: true, tag: tag || '#document' }};
                }}
                if (typeof el.focus === 'function') {{ try {{ el.focus(); }} catch (_) {{}} }}
                el.click();
                return {{ found: true, not_clickable: false }};
            }})()"#
        );
        runtime_evaluate(&session, &cdp_sid, &expr, timeout).await
    };

    match outcome {
        Err(e) => Ok(err_result(&e)),
        Ok(EvalOutcome::Threw(msg)) => Ok(err_result(&ErrorResponse::new(
            ErrorCode::CdpError,
            format!("click threw: {msg}"),
        ))),
        Ok(EvalOutcome::Ok { value, .. }) => {
            // Selector path returns {found, not_clickable, ...}; ref path
            // returns {not_clickable, ...}. Both are mapped here.
            if let Some(v) = value.as_ref() {
                if v.get("found").and_then(|f| f.as_bool()) == Some(false) {
                    return Ok(err_result(&ErrorResponse::new(
                        ErrorCode::ElementNotFound,
                        format!(
                            "no element matched selector {:?}",
                            input.selector.as_deref().unwrap_or("")
                        ),
                    )));
                }
                if v.get("not_clickable").and_then(|f| f.as_bool()) == Some(true) {
                    let tag = v.get("tag").and_then(|t| t.as_str()).unwrap_or("(unknown)");
                    return Ok(err_result(&ErrorResponse::new(
                        ErrorCode::NodeNotClickable,
                        format!(
                            "element <{tag}> is not a click target — pick a more specific ref \
                             (button, link, or interactive element) from the latest tree snapshot"
                        ),
                    )));
                }
            }
            let mut payload = ToolResponse::new(
                &session.short_id,
                session.last_url().await,
                ClickData { clicked: true },
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
