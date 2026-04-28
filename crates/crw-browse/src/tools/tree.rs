//! `tree` — snapshot the page as an indented accessibility tree.

use std::time::Instant;

use rmcp::{ErrorData as McpError, model::CallToolResult, schemars};
use serde::{Deserialize, Serialize};

use crate::errors::{ErrorCode, ErrorResponse};
use crate::response::ToolResponse;
use crate::server::CrwBrowse;
use crate::snapshot;
use crate::tools::common::{
    MAX_TREE_NODES, clamp_max_nodes, err_result, no_session_err, no_target_err, ok_result,
};

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct TreeInput {
    /// Maximum number of nodes to include in the output (default: 500).
    #[serde(default)]
    pub max_nodes: Option<u32>,
    /// Output format: `"text"` (default, indented `@e<N>` listing) or `"json"`
    /// (structured node array under `data.tree_json`). The text variant is
    /// always populated so existing callers keep working unchanged.
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TreeData {
    pub node_count: usize,
    pub tree: String,
    /// Structured tree, populated when `format: "json"` is requested. Each
    /// node carries the same `@e<N>` ref as the text rendering.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tree_json: Option<Vec<crate::snapshot::TreeNode>>,
}

pub async fn handle(server: &CrwBrowse, input: TreeInput) -> Result<CallToolResult, McpError> {
    let started = Instant::now();
    let (max_nodes_u32, max_nodes_clamped) = clamp_max_nodes(input.max_nodes);
    let max_nodes = max_nodes_u32 as usize;

    let Some(session) = server.default_session_get().await else {
        return Ok(err_result(&no_session_err()));
    };
    let Some(cdp_sid) = session.cdp_session_id().await else {
        return Ok(err_result(&no_target_err()));
    };

    let ax = match session
        .conn
        .send_recv(
            "Accessibility.getFullAXTree",
            serde_json::json!({}),
            Some(&cdp_sid),
            server.config().page_timeout,
        )
        .await
    {
        Ok(v) => v,
        Err(e) => {
            return Ok(err_result(&ErrorResponse::new(
                ErrorCode::CdpError,
                format!("Accessibility.getFullAXTree failed: {e}"),
            )));
        }
    };

    let nodes = ax.get("nodes").cloned().unwrap_or(serde_json::Value::Null);
    let node_count = nodes.as_array().map(|a| a.len()).unwrap_or(0);

    // Validate format up-front; anything other than `text`/`json`/None is a
    // typo we'd rather surface than silently fall back. Lower-cased so callers
    // can pass "JSON" or "Text".
    let want_json = match input.format.as_deref().map(str::to_lowercase).as_deref() {
        None | Some("text") => false,
        Some("json") => true,
        Some(other) => {
            return Ok(err_result(&ErrorResponse::new(
                ErrorCode::InvalidArgs,
                format!("format '{other}' invalid — expected 'text' or 'json'"),
            )));
        }
    };

    // Always render the text form — `tree` field is non-optional in TreeData
    // and existing callers depend on it. JSON is purely additive.
    let rendered = snapshot::render_compact(&nodes, max_nodes);
    let entries: Vec<(String, Option<i64>)> = rendered
        .refs
        .iter()
        .map(|e| (e.ref_id.clone(), e.backend_node_id))
        .collect();
    // Replace the session's ref_map atomically so subsequent click/fill calls
    // can resolve `@e<N>` refs from this snapshot. Old refs become stale.
    session.replace_ref_map(entries).await;

    let tree_json = if want_json {
        Some(snapshot::render_json(&nodes, max_nodes).roots)
    } else {
        None
    };

    let mut payload = ToolResponse::new(
        &session.short_id,
        session.last_url().await,
        TreeData {
            node_count,
            tree: rendered.text,
            tree_json,
        },
    )
    .with_elapsed_ms(started.elapsed().as_millis() as u64);
    if max_nodes_clamped {
        payload = payload.with_warning(format!(
            "max_nodes clamped to {MAX_TREE_NODES} (server-side cap)"
        ));
    }
    if node_count > max_nodes {
        payload = payload.with_warning(format!(
            "tree truncated: {node_count} nodes in AX, showing first {max_nodes}"
        ));
    }
    Ok(ok_result(&payload))
}
