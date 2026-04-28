//! `evaluate` — run an arbitrary JavaScript expression on the page and return
//! the result. Wraps `Runtime.evaluate` with `returnByValue=true` and
//! `awaitPromise=true` so async expressions resolve cleanly.

use std::time::Instant;

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
pub struct EvaluateInput {
    /// JavaScript expression to evaluate. May `await` promises.
    pub expression: String,
    /// Evaluate timeout in milliseconds (default: 30000, capped at 120000).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct EvaluateData {
    /// Primitive or JSON-serialisable return value. Present when the
    /// expression resolved to a value `Runtime.evaluate` could ship by-value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    /// CDP `description` string — populated for non-primitive returns
    /// (functions, DOM nodes, errors) where `value` is absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

pub async fn handle(server: &CrwBrowse, input: EvaluateInput) -> Result<CallToolResult, McpError> {
    let started = Instant::now();
    if input.expression.trim().is_empty() {
        return Ok(err_result(&ErrorResponse::new(
            ErrorCode::InvalidArgs,
            "expression must not be empty",
        )));
    }
    let (timeout, timeout_clamped) = clamp_timeout(input.timeout_ms, server.config().page_timeout);

    let Some(session) = server.default_session_get().await else {
        return Ok(err_result(&no_session_err()));
    };
    let Some(cdp_sid) = session.cdp_session_id().await else {
        return Ok(err_result(&no_target_err()));
    };

    match runtime_evaluate(&session, &cdp_sid, &input.expression, timeout).await {
        Err(e) => Ok(err_result(&e)),
        Ok(EvalOutcome::Threw(msg)) => Ok(err_result(&ErrorResponse::new(
            ErrorCode::InvalidExpression,
            format!("expression threw: {msg}"),
        ))),
        Ok(EvalOutcome::Ok { value, description }) => {
            let mut payload = ToolResponse::new(
                &session.short_id,
                session.last_url().await,
                EvaluateData { value, description },
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
