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
            ],
        },
        search: SearchCapabilities {
            answer: true,
            summarize_results: true,
        },
    })
}
