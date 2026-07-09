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
use crw_core::types::{OutputFormat, ScrapeRequest};

use super::adapters::expires_at_rfc3339;
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

    let mut valid = Vec::with_capacity(req.urls.len());
    for u in &req.urls {
        let parsed = url::Url::parse(u)
            .map_err(|e| CrwError::InvalidRequest(format!("Invalid URL {u}: {e}")))?;
        crw_core::url_safety::validate_safe_url_resolved(&parsed)
            .await
            .map_err(CrwError::InvalidRequest)?;
        valid.push(u.clone());
    }

    let template = ScrapeRequest {
        formats: vec![OutputFormat::Json],
        json_schema: req.schema.clone(),
        // A free-text extraction prompt (no schema) rides the summary_prompt slot,
        // which the extractor folds into the structured-extraction instruction.
        summary_prompt: req.prompt.clone(),
        ..Default::default()
    };

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
    let rec = {
        let jobs = state.extract_jobs.read().await;
        jobs.get(&id)
            .cloned()
            .ok_or_else(|| CrwError::NotFound(format!("Extract job {id} not found")))?
    };
    let expires_at = expires_at_rfc3339(rec.created_at, state.config.crawler.job_ttl_secs);
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
