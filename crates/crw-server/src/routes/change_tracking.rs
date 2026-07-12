//! `POST /v1/change-tracking/diff` — stateless change-tracking diff endpoint.
//!
//! This is the crawl-path workhorse: the SaaS monitor reconciler scrapes pages
//! (via `/v1/crawl`), then calls this endpoint with each page's current
//! markdown/json plus the prior snapshot to get a per-page diff. opencore
//! stores nothing — `previous` is supplied by the caller.
//!
//! Two wire shapes on one route, discriminated by the presence of the `batch`
//! key (no `deny_unknown_fields`, so a Single body's extra fields and a Batch
//! body's shared fields never reject each other):
//!   - Single: `{ current, previous?, modes, schema?, prompt?, contentType?, tag? }`
//!   - Batch:  `{ batch: [ { url?, current, previous?, ... } ], modes, schema?, ... }`
//!     where top-level `modes/schema/prompt/contentType` are shared defaults
//!     each item may override.
//!
//! `goal` / `judgeEnabled` are accepted and ignored: this endpoint is
//! deterministic and never calls an LLM. The judge runs only on `/v1/scrape`,
//! opt-in via `goal` + `judgeEnabled: true` alongside the `changeTracking`
//! format.

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use crw_core::error::CrwError;
use crw_core::types::{
    ApiResponse, ChangeTrackingMode, ChangeTrackingOptions, ChangeTrackingResult,
    ChangeTrackingSnapshot,
};
use serde::Deserialize;
use serde_json::Value;

use crate::error::AppError;
use crate::state::AppState;

/// The current scrape content for one page.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffCurrent {
    #[serde(default)]
    pub markdown: Option<String>,
    #[serde(default)]
    pub json: Option<Value>,
}

/// One page to diff (single body, or one entry of a batch).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffItem {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub current: Option<DiffCurrent>,
    #[serde(default)]
    pub previous: Option<ChangeTrackingSnapshot>,
    #[serde(default)]
    pub modes: Option<Vec<ChangeTrackingMode>>,
    #[serde(default)]
    pub schema: Option<Value>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default, alias = "content_type")]
    pub content_type: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    // Accepted and ignored: this endpoint is deterministic and never calls an
    // LLM. The judge runs only on /v1/scrape (goal + judgeEnabled: true).
    #[serde(default)]
    pub goal: Option<String>,
    #[serde(default, alias = "judge_enabled")]
    pub judge_enabled: Option<bool>,
}

/// Request body. The presence of `batch` selects batch mode. Single-mode
/// fields are flattened onto the same struct; in batch mode `modes/schema/
/// prompt/contentType` act as shared defaults for items that omit them.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffRequest {
    #[serde(default)]
    pub batch: Option<Vec<DiffItem>>,
    // ---- single-mode (and batch shared-default) fields ----
    #[serde(default)]
    pub current: Option<DiffCurrent>,
    #[serde(default)]
    pub previous: Option<ChangeTrackingSnapshot>,
    #[serde(default)]
    pub modes: Option<Vec<ChangeTrackingMode>>,
    #[serde(default)]
    pub schema: Option<Value>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default, alias = "content_type")]
    pub content_type: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub goal: Option<String>,
    #[serde(default, alias = "judge_enabled")]
    pub judge_enabled: Option<bool>,
}

fn default_modes() -> Vec<ChangeTrackingMode> {
    vec![ChangeTrackingMode::GitDiff]
}

/// Build options + run the diff for one item, applying shared defaults.
fn diff_one(
    item: &DiffItem,
    shared_modes: &Option<Vec<ChangeTrackingMode>>,
    shared_schema: &Option<Value>,
    shared_prompt: &Option<String>,
    shared_content_type: &Option<String>,
) -> Result<ChangeTrackingResult, CrwError> {
    let current = item.current.as_ref().ok_or_else(|| {
        CrwError::InvalidRequest("each diff item requires a 'current' object".into())
    })?;

    let modes = item
        .modes
        .clone()
        .or_else(|| shared_modes.clone())
        .unwrap_or_else(default_modes);

    let opts = ChangeTrackingOptions {
        modes,
        schema: item.schema.clone().or_else(|| shared_schema.clone()),
        prompt: item.prompt.clone().or_else(|| shared_prompt.clone()),
        previous: item.previous.clone(),
        tag: item.tag.clone(),
        content_type: item
            .content_type
            .clone()
            .or_else(|| shared_content_type.clone()),
    };

    let markdown = current.markdown.as_deref().unwrap_or("");
    Ok(crw_diff::compute_change_tracking(
        &opts,
        markdown,
        current.json.as_ref(),
        opts.content_type.as_deref(),
    ))
}

pub async fn diff(
    State(_state): State<AppState>,
    body: Result<Json<DiffRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<Value>>, AppError> {
    let Json(req) = body.map_err(AppError::from)?;

    // Batch mode: presence of `batch` wins.
    if let Some(items) = &req.batch {
        if items.is_empty() {
            return Err(AppError::from(CrwError::InvalidRequest(
                "'batch' must contain at least one item".into(),
            )));
        }
        let mut results: Vec<ChangeTrackingResult> = Vec::with_capacity(items.len());
        for item in items {
            results.push(diff_one(
                item,
                &req.modes,
                &req.schema,
                &req.prompt,
                &req.content_type,
            )?);
        }
        let data = serde_json::to_value(results)
            .map_err(|e| CrwError::Internal(format!("failed to serialize diff results: {e}")))?;
        return Ok(Json(ApiResponse::ok(data)));
    }

    // Single mode.
    let single = DiffItem {
        url: None,
        current: req.current.clone(),
        previous: req.previous.clone(),
        modes: req.modes.clone(),
        schema: req.schema.clone(),
        prompt: req.prompt.clone(),
        content_type: req.content_type.clone(),
        tag: req.tag.clone(),
        goal: req.goal.clone(),
        judge_enabled: req.judge_enabled,
    };
    let result = diff_one(&single, &None, &None, &None, &None)?;
    let data = serde_json::to_value(result)
        .map_err(|e| CrwError::Internal(format!("failed to serialize diff result: {e}")))?;
    Ok(Json(ApiResponse::ok(data)))
}
