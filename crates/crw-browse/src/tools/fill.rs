//! `fill` — set an input element's value and dispatch the standard
//! `input` + `change` events so framework listeners (React, Vue, ...) see
//! the update.
//!
//! Lightpanda's `Input.insertText` returns success but does not actually
//! mutate `element.value` (verified during T1 functional re-probe), and its
//! `DOM.focus` is missing entirely. So both backends use the same
//! `Runtime`-based path: assign `value`, then `dispatchEvent`. That's
//! identical to what testing libraries (Testing Library, Cypress) do under
//! the hood, so framework code paths fire correctly.

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

/// Function body used on both code paths. Sets `value`, dispatches `input`
/// and `change` (both bubbling), and returns the post-write value so the
/// caller can verify the assignment took. Defined once to keep selector
/// and ref paths identical.
const FILL_FN: &str = r#"function(v) {
    this.value = v;
    this.dispatchEvent(new Event('input', { bubbles: true }));
    this.dispatchEvent(new Event('change', { bubbles: true }));
    return this.value;
}"#;

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct FillInput {
    /// CSS selector. Mutually exclusive with `ref`.
    #[serde(default)]
    pub selector: Option<String>,
    /// `@e<N>` ref from the most recent `tree` snapshot. Mutually exclusive
    /// with `selector`.
    #[serde(default, rename = "ref")]
    pub ref_id: Option<String>,
    /// Value to assign to the element.
    pub value: String,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct FillData {
    /// Post-write `element.value`. Allows the LLM to verify the assignment
    /// succeeded — frameworks that proxy `value` via setters can reject the
    /// write, in which case this will differ from `value`.
    pub value: String,
}

pub async fn handle(server: &CrwBrowse, input: FillInput) -> Result<CallToolResult, McpError> {
    let started = Instant::now();
    let (timeout, timeout_clamped) = clamp_timeout(input.timeout_ms, server.config().page_timeout);

    // Empty-string selector would reach `document.querySelector("")` and
    // throw a `SyntaxError` from CDP, surfacing as `CDP_ERROR`. Other tools
    // (`goto {url:""}`, `type_text {text:""}`, every `wait` variant) reject
    // empty inputs as `INVALID_ARGS`; the shared validator mirrors that
    // contract so callers get a consistent shape.
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
        // CDP `Runtime.callFunctionOn` arguments are an array of
        // `CallArgument` objects, each with a `value` field that CDP unwraps
        // to a primitive before binding it to the function's parameters. So
        // `[{"value": v}]` is the correct shape — the page-side function
        // receives `v` directly, not the wrapper. Spec:
        // https://chromedevtools.github.io/devtools-protocol/tot/Runtime/#type-CallArgument
        let result = call_function_on(
            &session,
            &cdp_sid,
            &object_id,
            FILL_FN,
            serde_json::json!([{ "value": input.value }]),
            timeout,
        )
        .await;
        release_object_id(&session, &cdp_sid, &object_id, timeout).await;
        result
    } else {
        let Some(sel) = input.selector.as_deref() else {
            unreachable!("guarded by selector/ref XOR check above")
        };
        let sel_json = serde_json::to_string(sel).unwrap_or_else(|_| "\"\"".into());
        let val_json = serde_json::to_string(&input.value).unwrap_or_else(|_| "\"\"".into());
        // Inlines `FILL_FN` against `el` for the selector path. Equivalent to
        // calling `(FILL_FN).call(el, value)`.
        let expr = format!(
            r#"(() => {{
                const el = document.querySelector({sel_json});
                if (!el) return {{ found: false }};
                const v = {val_json};
                el.value = v;
                el.dispatchEvent(new Event('input', {{ bubbles: true }}));
                el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                return {{ found: true, value: el.value }};
            }})()"#
        );
        runtime_evaluate(&session, &cdp_sid, &expr, timeout).await
    };

    match outcome {
        Err(e) => Ok(err_result(&e)),
        Ok(EvalOutcome::Threw(msg)) => Ok(err_result(&ErrorResponse::new(
            ErrorCode::CdpError,
            format!("fill threw: {msg}"),
        ))),
        Ok(EvalOutcome::Ok { value, .. }) => {
            // ref path returns the bare string; selector path returns
            // {found, value}. Normalise both.
            let post_value = match value.as_ref() {
                Some(v) if v.is_string() => v.as_str().unwrap_or("").to_string(),
                Some(v) if v.is_object() => {
                    if v.get("found").and_then(|f| f.as_bool()) == Some(false) {
                        return Ok(err_result(&ErrorResponse::new(
                            ErrorCode::ElementNotFound,
                            format!(
                                "no element matched selector {:?}",
                                input.selector.as_deref().unwrap_or("")
                            ),
                        )));
                    }
                    v.get("value")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string()
                }
                _ => String::new(),
            };

            let mut payload = ToolResponse::new(
                &session.short_id,
                session.last_url().await,
                FillData { value: post_value },
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
