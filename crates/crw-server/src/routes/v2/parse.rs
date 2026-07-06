//! `POST /v2/parse` — upload a document (PDF) and get markdown back.
//!
//! Firecrawl-compatible file-parsing endpoint. Accepts `multipart/form-data`
//! with a `file` part (the PDF bytes) and an optional `options` part (a JSON
//! string mirroring the `/v2/scrape` body — `formats`, `jsonSchema`, …).
//! Returns the same `{ success, data, warning? }` envelope as `/v2/scrape`.
//!
//! Memory safety: the route carries its own 50 MB `DefaultBodyLimit` (wired in
//! `v2/mod.rs`) — far above the global 1 MB JSON cap but bounded — and a
//! concurrency semaphore so N simultaneous uploads can't exhaust memory.

use std::sync::{Arc, OnceLock};

use axum::Json;
use axum::extract::{Multipart, State};
use serde::Deserialize;
use tokio::sync::Semaphore;
use uuid::Uuid;

use crw_core::error::CrwError;
use crw_core::types::{OutputFormat, ParserSpec, ScrapeRequest};
use crw_crawl::pdf::{PdfSource, apply_llm_formats, convert_pdf_bytes_strict};

use super::adapters::to_v2_document;
use super::formats::{self, FormatSpec, decompose};
use super::scrape::V2ScrapeResponse;
use crate::error::AppError;
use crate::state::AppState;

/// Hard request-body cap for `/v2/parse` (50 MiB), applied as a per-route
/// `DefaultBodyLimit` in `v2/mod.rs`. Matches the renderer's response cap and
/// the default `[document].max_upload_bytes`.
pub const MAX_UPLOAD_BYTES: usize = 52_428_800;

/// Bounds the number of uploads parsed concurrently. Initialized lazily from
/// `[document].upload_concurrency` on first request.
static UPLOAD_SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();

fn upload_slots(state: &AppState) -> Arc<Semaphore> {
    UPLOAD_SLOTS
        .get_or_init(|| {
            let n = state.config.document.upload_concurrency.max(1);
            Arc::new(Semaphore::new(n))
        })
        .clone()
}

/// Subset of the v2 scrape body accepted via the `options` multipart field.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ParseOptions {
    formats: Option<Vec<FormatSpec>>,
    parsers: Option<Vec<ParserSpec>>,
    #[serde(alias = "json_schema")]
    json_schema: Option<serde_json::Value>,
    summary_prompt: Option<String>,
    max_content_chars: Option<usize>,
}

pub async fn parse(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<V2ScrapeResponse>, AppError> {
    if !state.config.document.enabled {
        return Err(AppError::from(CrwError::ExtractionError(
            "document parsing is disabled on this server ([document] enabled = false)".into(),
        )));
    }

    // Bound concurrent in-memory uploads.
    let slots = upload_slots(&state);
    let _permit = slots
        .acquire_owned()
        .await
        .map_err(|_| CrwError::Internal("upload semaphore closed".into()))?;

    // ── Read the multipart form ─────────────────────────────────────────────
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut filename: Option<String> = None;
    let mut options_raw: Option<String> = None;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        // 413 surfaces from the body-limit layer before we get here; other
        // multipart errors are malformed-request (400).
        CrwError::InvalidRequest(format!("invalid multipart form: {e}"))
    })? {
        match field.name() {
            Some("file") => {
                filename = field.file_name().map(|s| s.to_string());
                let data = field
                    .bytes()
                    .await
                    .map_err(|e| CrwError::InvalidRequest(format!("failed to read file: {e}")))?;
                file_bytes = Some(data.to_vec());
            }
            Some("options") => {
                options_raw = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| CrwError::InvalidRequest(format!("invalid options: {e}")))?,
                );
            }
            // Ignore unknown parts (forward-compat with newer SDKs).
            _ => {
                let _ = field.bytes().await;
            }
        }
    }

    let bytes = file_bytes
        .ok_or_else(|| CrwError::InvalidRequest("multipart form missing 'file' part".into()))?;

    // Magic-byte sniff: pdf-inspector only does PDF, so reject anything else up
    // front with a clear 400 rather than a downstream parse error.
    if !looks_like_pdf(&bytes) {
        return Err(AppError::from(CrwError::InvalidRequest(
            "uploaded file is not a PDF (missing %PDF- header); only PDF is supported".into(),
        )));
    }

    // ── Build the internal request from options ─────────────────────────────
    let opts: ParseOptions = match options_raw.as_deref() {
        Some(raw) if !raw.trim().is_empty() => serde_json::from_str(raw)
            .map_err(|e| CrwError::InvalidRequest(format!("invalid options JSON: {e}")))?,
        _ => ParseOptions::default(),
    };

    let format_specs = opts
        .formats
        .unwrap_or_else(|| vec![FormatSpec::String("markdown".to_string())]);
    let decomposed = decompose(&format_specs).map_err(CrwError::InvalidRequest)?;

    let req = ScrapeRequest {
        url: String::new(),
        formats: decomposed.formats.clone(),
        json_schema: opts.json_schema.or(decomposed.json_schema.clone()),
        summary_prompt: opts.summary_prompt,
        max_content_chars: opts.max_content_chars,
        parsers: opts.parsers,
        ..Default::default()
    };

    let llm_config = state.config.extraction.llm.as_ref();
    if req.formats.contains(&OutputFormat::Summary) && llm_config.is_none() {
        return Err(AppError::from(CrwError::InvalidRequest(
            "summary format requires LLM config: set [extraction.llm] in server config".into(),
        )));
    }

    let source = PdfSource {
        source_url: format!("upload://{}", filename.as_deref().unwrap_or("document.pdf")),
        status_code: 200,
        elapsed_ms: 0,
        source_filename: filename,
    };

    // Strict: a bad/encrypted/corrupt upload is a hard error (the user handed
    // us the file directly), unlike the URL path which soft-warns.
    let mut data = convert_pdf_bytes_strict(bytes, &req, source)
        .await
        .map_err(|(crw_err, _pdf_err)| AppError::from(crw_err))?;

    // Run LLM-backed formats (json/summary) on the extracted markdown.
    apply_llm_formats(&mut data, &req, llm_config).await?;

    let warning = formats::unsupported_warning(&decomposed.unsupported);
    let doc = to_v2_document(data, "basic", Uuid::new_v4().to_string());
    Ok(Json(V2ScrapeResponse {
        success: true,
        data: doc,
        warning,
        // PDF upload path is the content itself, never an anti-bot shell.
        error: None,
    }))
}

/// True when the buffer begins with the PDF magic header (after an optional
/// UTF-8 BOM / leading whitespace, mirroring lopdf's own tolerance).
fn looks_like_pdf(bytes: &[u8]) -> bool {
    let b = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let start = b
        .iter()
        .position(|&c| !c.is_ascii_whitespace())
        .unwrap_or(b.len());
    b[start..].starts_with(b"%PDF-")
}
