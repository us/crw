//! `GET /v1/capabilities` — surface what this opencore instance supports.
//!
//! SaaS / dashboard frontends call this on boot to decide which provider
//! buttons / formats to surface. Closes the "SaaS UI shipped before
//! opencore rollout" silent-failure mode by giving callers a way to ask
//! "do you actually do this?" before making a real request.

use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::state::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub version: &'static str,
    pub llm: LlmCapabilities,
    pub formats: FormatCapabilities,
    pub search: SearchCapabilities,
    pub extract: ExtractCapabilities,
    pub documents: DocumentCapabilities,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractCapabilities {
    /// Native `POST /v1/extract` (async multi-URL structured extraction) is live.
    pub supported: bool,
    /// Max URLs accepted per request (`crawler.max_extract_urls`).
    pub max_urls: usize,
    /// Per-field `basis` attribution (Phase 2b). False until 2b ships.
    pub per_field_attribution: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentCapabilities {
    /// Document parser types this instance can apply. `["pdf"]` when PDF
    /// support is compiled in and enabled; empty otherwise. The SaaS gates the
    /// `parsers` option and the upload UI on this.
    pub parsers: Vec<&'static str>,
    /// File-upload (`POST /v2/parse`) availability + limits.
    pub file_upload: FileUploadCapabilities,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileUploadCapabilities {
    pub supported: bool,
    pub endpoint: &'static str,
    pub max_bytes: usize,
    pub types: Vec<&'static str>,
    /// pdf-inspector has no OCR — scanned/image PDFs yield empty/partial text.
    pub ocr: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmCapabilities {
    /// Provider tags the server's dispatch knows about.
    pub providers: Vec<&'static str>,
    pub supports_base_url: bool,
    /// True when a server-wide LLM key is configured (self-hosted /
    /// no-SaaS deploys). SaaS-fronted deploys set
    /// `CRW_DISABLE_SERVER_LLM_KEY=1` and rely on per-request BYOK.
    pub server_key_configured: bool,
    /// Configured server-side fan-out cap for LLM calls. 0 when no
    /// server-side LLM config is present.
    pub max_concurrency: usize,
    /// Header name the server will look for on LLM-touching requests
    /// (`None` means no header guard).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_byok_header: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatCapabilities {
    pub supported: Vec<&'static str>,
    /// Change-tracking diff modes this instance supports. Empty when the
    /// `changeTracking` format is unavailable. The SaaS capability-gate checks
    /// `supported` contains `"changeTracking"` before emitting monitor scrapes.
    pub change_tracking_modes: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchCapabilities {
    pub answer: bool,
    pub summarize_results: bool,
}

pub async fn capabilities(State(state): State<AppState>) -> Json<Capabilities> {
    let llm_cfg = state.config.extraction.llm.as_ref();
    Json(Capabilities {
        version: env!("CARGO_PKG_VERSION"),
        llm: LlmCapabilities {
            providers: vec![
                "anthropic",
                "openai",
                "deepseek",
                "openai-compatible",
                "azure",
            ],
            supports_base_url: true,
            server_key_configured: llm_cfg.map(|c| !c.api_key.is_empty()).unwrap_or(false),
            max_concurrency: llm_cfg.map(|c| c.max_concurrency).unwrap_or(0),
            require_byok_header: llm_cfg.and_then(|c| c.require_byok_header.clone()),
        },
        formats: FormatCapabilities {
            supported: vec![
                "markdown",
                "html",
                "rawHtml",
                "plainText",
                "links",
                "json",
                "summary",
                "changeTracking",
            ],
            change_tracking_modes: vec!["gitDiff", "json"],
        },
        search: SearchCapabilities {
            answer: true,
            summarize_results: true,
        },
        extract: ExtractCapabilities {
            supported: true,
            max_urls: state.config.crawler.max_extract_urls,
            per_field_attribution: false,
        },
        documents: {
            let pdf_on = crw_extract::pdf::PDF_SUPPORTED && state.config.document.enabled;
            DocumentCapabilities {
                parsers: if pdf_on { vec!["pdf"] } else { vec![] },
                file_upload: FileUploadCapabilities {
                    supported: pdf_on,
                    endpoint: "/v2/parse",
                    max_bytes: state.config.document.max_upload_bytes,
                    types: if pdf_on {
                        vec!["application/pdf"]
                    } else {
                        vec![]
                    },
                    ocr: false,
                },
            }
        },
    })
}
