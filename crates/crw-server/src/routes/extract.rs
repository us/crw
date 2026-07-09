//! Native `POST /v1/extract` (+ `GET /v1/extract/{id}`). Multi-URL structured
//! extraction as an async job. Unlike the FC-legacy `/v2/extract` (which merges
//! every URL's JSON into one object, last-write-wins), the native route returns
//! a **per-URL array** (`results:[{url,status,data,error,llmUsage}]`) that keeps
//! each URL's object distinct and carries per-URL LLM usage for downstream
//! billing. No FC envelope (`success`/`urlTrace`/deprecation warning).

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crw_core::error::CrwError;
use crw_core::types::{ExtractOptions, LlmUsage, OutputFormat, ScrapeRequest};

use crate::error::AppError;
use crate::routes::v2::adapters::expires_at_rfc3339;
use crate::state::{AppState, PreparedUrl, UrlResult};

/// Native extract request. camelCase like every other v1 public type.
/// NOTE: no `#[derive(Debug)]` — `llm_api_key` is a secret and must never land
/// in a `{:?}` log line.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractRequest {
    #[serde(default)]
    pub urls: Vec<String>,
    /// Free-text extraction objective (LLM infers the shape). Wired into the
    /// extractor's `extract.prompt` slot — the field JSON extraction actually
    /// reads (NOT `summary_prompt`, which only drives the summary format).
    #[serde(default)]
    pub prompt: Option<String>,
    /// JSON Schema constraining the output.
    #[serde(default)]
    pub schema: Option<Value>,
    // BYOK passthrough.
    #[serde(default)]
    pub llm_api_key: Option<String>,
    #[serde(default)]
    pub llm_provider: Option<String>,
    #[serde(default)]
    pub llm_model: Option<String>,
    // `base_url` is parsed only so we can REJECT it with a clear 400 instead of
    // silently ignoring it (which would route a BYOK key to the wrong endpoint).
    // It flows unvalidated into the LLM client (`build_byok_llm_config`), an SSRF
    // vector shared engine-wide; not accepted here until that path validates it.
    #[serde(default)]
    pub base_url: Option<String>,
    /// Per-field attribution (Phase 2b). Parsed now; until 2b ships, `true` is
    /// rejected so callers never believe an inert flag is honored.
    #[serde(default)]
    pub basis: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractStartResponse {
    pub id: String,
    pub status: String,
    /// Count of URLs actually enqueued for fetch (preflight-failed URLs are in
    /// the status `results`, not this count).
    pub urls: usize,
}

pub async fn start_extract(
    State(state): State<AppState>,
    body: Result<Json<ExtractRequest>, JsonRejection>,
) -> Result<Json<ExtractStartResponse>, AppError> {
    let Json(req) = body.map_err(AppError::from)?;
    let prepared = prepare_extract(&state, req).await?;
    let urls = prepared.valid_count;
    let id = state
        .start_extract_job(prepared.entries, prepared.template)
        .await;
    Ok(Json(ExtractStartResponse {
        id: id.to_string(),
        status: "processing".to_string(),
        urls,
    }))
}

/// Validated + SSRF-preflighted extract inputs, ready for `start_extract_job`.
pub(crate) struct PreparedExtract {
    pub entries: Vec<PreparedUrl>,
    pub template: ScrapeRequest,
    /// Count of URLs enqueued for fetch (preflight-failed URLs excluded).
    pub valid_count: usize,
}

/// Shared validation, SSRF preflight, and template build for the HTTP route and
/// the MCP `crw_extract` tool. Returns `CrwError::InvalidRequest` (→ 400) on any
/// rejected input so both callers get identical semantics.
pub(crate) async fn prepare_extract(
    state: &AppState,
    req: ExtractRequest,
) -> Result<PreparedExtract, CrwError> {
    if req.urls.is_empty() {
        return Err(CrwError::InvalidRequest(
            "`urls` is required and must be non-empty".into(),
        ));
    }
    let cap = state.config.crawler.max_extract_urls;
    if req.urls.len() > cap {
        return Err(CrwError::InvalidRequest(format!(
            "too many urls: {} exceeds the per-request limit of {cap}",
            req.urls.len()
        )));
    }
    // A whitespace-only prompt is treated as absent (the extractor filters it to
    // empty anyway) so we reject upfront instead of fetching then failing.
    let has_prompt = req.prompt.as_deref().is_some_and(|p| !p.trim().is_empty());
    if !has_prompt && req.schema.is_none() {
        return Err(CrwError::InvalidRequest(
            "nothing to extract: provide a non-empty `prompt`, a `schema`, or both".into(),
        ));
    }
    if req.basis == Some(true) {
        return Err(CrwError::InvalidRequest(
            "`basis` (per-field attribution) is not yet supported".into(),
        ));
    }
    if req.base_url.is_some() {
        return Err(CrwError::InvalidRequest(
            "`baseUrl` is not supported on /v1/extract; configure the LLM endpoint \
             server-side ([extraction.llm.base_url])"
                .into(),
        ));
    }

    // LLM-availability guards, upfront (cheaper than failing in the worker).
    // Mirror /v1/scrape's BYOK-header guard: the worker reaches the LLM directly,
    // bypassing the scrape handler's check.
    if let Some(cfg) = state.config.extraction.llm.as_ref()
        && cfg.require_byok_header.is_some()
        && req.llm_api_key.is_none()
    {
        return Err(CrwError::InvalidRequest(
            "LLM features require a per-request llm_api_key (BYOK header guard active)".into(),
        ));
    }
    if state.config.extraction.llm.is_none() && req.llm_api_key.is_none() {
        return Err(CrwError::InvalidRequest(
            "extraction requires an LLM: set [extraction.llm] in server config or pass \
             llm_api_key in the request body"
                .into(),
        ));
    }

    // Per-URL preflight in original order. Bad parse / SSRF failures become
    // `failed` results (surfaced, not dropped); valid URLs are enqueued.
    let mut entries = Vec::with_capacity(req.urls.len());
    let mut valid_count = 0usize;
    for u in &req.urls {
        match url::Url::parse(u) {
            Ok(parsed) => match crw_core::url_safety::validate_safe_url_resolved(&parsed).await {
                Ok(()) => {
                    valid_count += 1;
                    entries.push(PreparedUrl {
                        url: u.clone(),
                        preflight_error: None,
                    });
                }
                Err(e) => entries.push(PreparedUrl {
                    url: u.clone(),
                    preflight_error: Some(e),
                }),
            },
            Err(e) => entries.push(PreparedUrl {
                url: u.clone(),
                preflight_error: Some(format!("invalid URL: {e}")),
            }),
        }
    }
    if valid_count == 0 {
        return Err(CrwError::InvalidRequest(
            "no valid URLs to extract (all failed URL parsing or the SSRF safety check)".into(),
        ));
    }

    let template = ScrapeRequest {
        formats: vec![OutputFormat::Json],
        json_schema: req.schema.clone(),
        // `extract.prompt` is the field JSON extraction reads (single.rs).
        extract: Some(ExtractOptions {
            schema: None,
            prompt: req.prompt.clone(),
        }),
        llm_api_key: req.llm_api_key.clone(),
        llm_provider: req.llm_provider.clone(),
        llm_model: req.llm_model.clone(),
        ..Default::default()
    };

    Ok(PreparedExtract {
        entries,
        template,
        valid_count,
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractUrlResult {
    pub url: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_usage: Option<LlmUsage>,
}

impl From<UrlResult> for ExtractUrlResult {
    fn from(r: UrlResult) -> Self {
        ExtractUrlResult {
            url: r.url,
            status: r.status.as_str().to_string(),
            data: r.data,
            error: r.error,
            llm_usage: r.llm_usage,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractStatusResponse {
    pub id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub results: Vec<ExtractUrlResult>,
    /// Job-level error, set only when every URL failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub expires_at: String,
    pub credits_used: u32,
    pub tokens_used: u32,
}

pub async fn get_extract(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ExtractStatusResponse>, AppError> {
    let rec = {
        let jobs = state.extract_jobs.read().await;
        jobs.get(&id)
            .cloned()
            .ok_or_else(|| CrwError::NotFound(format!("Extract job {id} not found")))?
    };
    let expires_at = expires_at_rfc3339(rec.created_at, state.config.crawler.job_ttl_secs);
    Ok(Json(ExtractStatusResponse {
        id: id.to_string(),
        status: rec.status.as_str().to_string(),
        results: rec
            .per_url
            .into_iter()
            .map(ExtractUrlResult::from)
            .collect(),
        error: rec.error,
        expires_at,
        credits_used: rec.credits_used,
        tokens_used: rec.tokens_used,
    }))
}
