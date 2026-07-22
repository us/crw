//! `POST /v2/extract` (+ `GET /v2/extract/{id}`). Deprecated in Firecrawl v2
//! (the live API tells callers to use `/v2/scrape` with a `json` format). We
//! model it as an async job over the json-extract path: scrape each URL with
//! `formats:[json]` + the shared schema, merge the per-URL `json` objects into
//! a single object (the live API's `data` shape).

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crw_core::error::CrwError;
use crw_core::types::{ExtractOptions, OutputFormat, ScrapeRequest};

use super::adapters::system_time_rfc3339;
use crate::error::AppError;
use crate::state::{AppState, ExtractStatus, PreparedUrl};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct V2ExtractRequest {
    #[serde(default)]
    pub urls: Vec<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub schema: Option<Value>,
    #[serde(default)]
    pub system_prompt: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct V2ExtractStartResponse {
    pub success: bool,
    pub id: String,
    pub url_trace: Vec<Value>,
    pub warnings: Vec<String>,
    pub replacement: String,
}

/// Build the per-URL scrape template for an extract job.
///
/// The prompt has to ride `extract.prompt` — that is the field the JSON
/// extraction path reads (single.rs). It used to go into `summary_prompt`,
/// which only drives the `summary` format, so a prompt-only extract hit the
/// "requires a jsonSchema field or a prompt" error despite carrying one, and a
/// schema+prompt extract silently ignored the prompt.
pub(crate) fn extract_template(prompt: Option<String>, schema: Option<Value>) -> ScrapeRequest {
    ScrapeRequest {
        formats: vec![OutputFormat::Json],
        json_schema: schema,
        extract: Some(ExtractOptions {
            schema: None,
            prompt,
        }),
        ..Default::default()
    }
}

pub async fn start_extract(
    State(state): State<AppState>,
    body: Result<Json<V2ExtractRequest>, JsonRejection>,
) -> Result<Json<V2ExtractStartResponse>, AppError> {
    let Json(req) = body.map_err(AppError::from)?;
    if req.urls.is_empty() {
        return Err(AppError::from(CrwError::InvalidRequest(
            "`urls` is required for extract on this engine (prompt-only extraction \
             without URLs is not supported)"
                .into(),
        )));
    }
    // `systemPrompt` is a distinct Firecrawl feature (its own LLM `system`
    // role), not an alias for `prompt`, and the extractor has no slot for it
    // yet. Reject it loudly rather than accept it and silently ignore it —
    // the same contract the engine already gives `actions` (single.rs).
    if req
        .system_prompt
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty())
    {
        return Err(AppError::from(CrwError::InvalidRequest(
            "systemPrompt is not yet supported on this engine; fold your \
             instruction into `prompt`."
                .into(),
        )));
    }

    let mut valid = Vec::with_capacity(req.urls.len());
    for u in &req.urls {
        let parsed = url::Url::parse(u)
            .map_err(|e| CrwError::InvalidRequest(format!("Invalid URL {u}: {e}")))?;
        crw_core::url_safety::validate_safe_url_resolved(&parsed)
            .await
            .map_err(CrwError::InvalidRequest)?;
        valid.push(u.clone());
    }

    let template = extract_template(req.prompt.clone(), req.schema.clone());

    // v2 early-returns on the first bad URL (above), so every entry is valid.
    let entries = valid
        .into_iter()
        .map(|url| PreparedUrl {
            url,
            preflight_error: None,
        })
        .collect();
    let id = state.start_extract_job(entries, template).await;
    Ok(Json(V2ExtractStartResponse {
        success: true,
        id: id.to_string(),
        url_trace: vec![],
        warnings: vec![
            "/v2/extract is deprecated. Use /v2/scrape with formats including a 'json' \
             format object."
                .to_string(),
        ],
        replacement: "/v2/scrape".to_string(),
    }))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct V2ExtractStatusResponse {
    pub success: bool,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub expires_at: String,
    pub credits_used: u32,
    pub tokens_used: u32,
}

pub async fn get_extract(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<V2ExtractStatusResponse>, AppError> {
    let rec = state.get_extract_job(id).await?;
    let expires_at = system_time_rfc3339(rec.expires_at);
    Ok(Json(V2ExtractStatusResponse {
        success: !matches!(rec.status, ExtractStatus::Failed),
        status: rec.status.as_str().to_string(),
        data: rec.data,
        error: rec.error,
        expires_at,
        credits_used: rec.credits_used,
        tokens_used: rec.tokens_used,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The regression: the prompt must land in `extract.prompt`, which the JSON
    /// extraction path reads, NOT in `summary_prompt`, which only drives the
    /// summary format. Putting it in the wrong slot made a prompt-only extract
    /// fail with "requires a jsonSchema field or a prompt" despite the caller
    /// having supplied one.
    #[test]
    fn prompt_lands_in_extract_not_summary() {
        let t = extract_template(Some("Get the title".into()), None);
        assert_eq!(
            t.extract.as_ref().and_then(|e| e.prompt.as_deref()),
            Some("Get the title")
        );
        assert!(t.summary_prompt.is_none());
        assert!(t.formats.contains(&OutputFormat::Json));
    }

    #[test]
    fn schema_and_prompt_both_survive() {
        let schema = serde_json::json!({"type": "object"});
        let t = extract_template(Some("steer".into()), Some(schema.clone()));
        assert_eq!(t.json_schema.as_ref(), Some(&schema));
        assert_eq!(
            t.extract.as_ref().and_then(|e| e.prompt.as_deref()),
            Some("steer")
        );
    }
}
