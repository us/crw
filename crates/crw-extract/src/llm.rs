//! LLM provider dispatch (Anthropic, OpenAI, OpenAI-compatible, Azure).
//!
//! Two surfaces:
//!
//! * [`extract_via_llm`] — content-extraction fallback used when DOM-based
//!   extraction (readability + heuristics) yields a low-quality candidate.
//! * [`chat`] — generic single-turn chat call used by [`crate::summary`]
//!   and [`crate::answer`] for user-facing LLM features.
//!
//! All paths share one pooled [`reqwest::Client`] (per-call clients leak
//! TCP connections under load).

use crate::pricing;
use crw_core::config::LlmConfig;
use crw_core::error::{CrwError, CrwResult};
use crw_core::types::LlmUsage;
use std::sync::OnceLock;
use std::time::Duration;

const ANTHROPIC_DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1/messages";
const OPENAI_DEFAULT_BASE_URL: &str = "https://api.openai.com/v1/chat/completions";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

const EXTRACTION_SYSTEM_PROMPT: &str = "You extract the main article or content body from a web page's HTML. \
Return only the article text as plain markdown. Strip nav, header, footer, ads, comments, related links, \
sidebars, cookie banners, share buttons, author bios, social widgets. No preamble or commentary, no fenced \
code block — just the markdown content.";

/// Result of one LLM call: textual content + best-effort usage metadata.
#[derive(Debug, Clone)]
pub struct LlmCallResult {
    pub content: String,
    pub usage: Option<LlmUsage>,
    pub warning: Option<String>,
}

fn shared_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("reqwest client build (LLM shared)")
    })
}

fn truncate_on_char_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut idx = max_bytes;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    &s[..idx]
}

/// Content-extraction fallback. Returns just the markdown content for
/// backward compatibility with the existing readability pipeline.
#[allow(clippy::too_many_arguments)]
pub async fn extract_via_llm(
    html: &str,
    api_key: &str,
    provider: &str,
    model: &str,
    base_url: Option<&str>,
    max_tokens: u32,
    max_html_bytes: usize,
    azure_api_version: Option<&str>,
) -> CrwResult<String> {
    if api_key.is_empty() {
        return Err(CrwError::InvalidRequest(
            "LLM fallback enabled but api_key is empty".into(),
        ));
    }
    let truncated = truncate_on_char_boundary(html, max_html_bytes);
    let user_msg =
        format!("Extract the main article/content body from this HTML as markdown:\n\n{truncated}");

    let result = dispatch(
        provider,
        api_key,
        model,
        base_url,
        max_tokens,
        azure_api_version,
        EXTRACTION_SYSTEM_PROMPT,
        &user_msg,
    )
    .await?;
    Ok(result.content)
}

/// Generic single-turn chat call used by feature-level modules
/// ([`crate::summary`], [`crate::answer`]).
///
/// Uses `cfg.api_key/provider/model/base_url/max_tokens/azure_api_version`.
pub async fn chat(
    cfg: &LlmConfig,
    system_prompt: &str,
    user_msg: &str,
) -> CrwResult<LlmCallResult> {
    if cfg.api_key.is_empty() {
        return Err(CrwError::InvalidRequest(
            "LLM call requires non-empty api_key — set CRW_EXTRACTION__LLM__API_KEY \
             or pass llm_api_key in request"
                .into(),
        ));
    }
    dispatch(
        &cfg.provider,
        &cfg.api_key,
        &cfg.model,
        cfg.base_url.as_deref(),
        cfg.max_tokens,
        cfg.azure_api_version.as_deref(),
        system_prompt,
        user_msg,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn dispatch(
    provider: &str,
    api_key: &str,
    model: &str,
    base_url: Option<&str>,
    max_tokens: u32,
    azure_api_version: Option<&str>,
    system_prompt: &str,
    user_msg: &str,
) -> CrwResult<LlmCallResult> {
    let client = shared_client();
    match provider.to_ascii_lowercase().as_str() {
        "anthropic" => {
            call_anthropic(
                client,
                api_key,
                model,
                base_url,
                max_tokens,
                system_prompt,
                user_msg,
            )
            .await
        }
        // DeepSeek and other OpenAI-compatible providers use the same wire
        // protocol; users select them via `base_url`.
        "openai" | "deepseek" | "openai-compatible" => {
            call_openai(
                client,
                api_key,
                model,
                base_url,
                max_tokens,
                system_prompt,
                user_msg,
            )
            .await
        }
        "azure" => {
            let endpoint = base_url.ok_or_else(|| {
                CrwError::InvalidRequest(
                    "azure provider requires base_url (Azure OpenAI endpoint)".into(),
                )
            })?;
            let version = azure_api_version.ok_or_else(|| {
                CrwError::InvalidRequest(
                    "azure provider requires azure_api_version (e.g. 2024-05-01-preview)".into(),
                )
            })?;
            call_azure(
                client,
                api_key,
                endpoint,
                model,
                version,
                max_tokens,
                system_prompt,
                user_msg,
            )
            .await
        }
        other => Err(CrwError::InvalidRequest(format!(
            "unknown LLM provider: {other}. Supported: anthropic, openai, deepseek, azure"
        ))),
    }
}

#[allow(clippy::too_many_arguments)]
async fn call_anthropic(
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    base_url: Option<&str>,
    max_tokens: u32,
    system_prompt: &str,
    user_msg: &str,
) -> CrwResult<LlmCallResult> {
    let url = base_url.unwrap_or(ANTHROPIC_DEFAULT_BASE_URL);
    let body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "system": system_prompt,
        "messages": [{ "role": "user", "content": user_msg }],
    });
    let resp = client
        .post(url)
        .header("x-api-key", api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| CrwError::Internal(format!("LLM request failed: {e}")))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| CrwError::Internal(format!("LLM response read failed: {e}")))?;
    if !status.is_success() {
        // NOTE: body may contain the request echoed back by some gateways.
        // The HTTP status code is enough — do not leak the body.
        return Err(CrwError::Internal(format!(
            "LLM HTTP {status} from anthropic"
        )));
    }
    let payload: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| CrwError::Internal(format!("LLM response parse failed: {e}")))?;
    let content = payload
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| {
            arr.iter()
                .find_map(|b| b.get("text").and_then(|t| t.as_str()))
        })
        .map(|s| s.to_string())
        .ok_or_else(|| CrwError::Internal("anthropic response missing content".to_string()))?;

    let usage = parse_anthropic_usage(&payload, model);
    Ok(LlmCallResult {
        content,
        usage,
        warning: None,
    })
}

#[allow(clippy::too_many_arguments)]
async fn call_openai(
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    base_url: Option<&str>,
    max_tokens: u32,
    system_prompt: &str,
    user_msg: &str,
) -> CrwResult<LlmCallResult> {
    // Accept either a full endpoint URL or a `…/v1` base; append the path if
    // missing so users don't have to remember the suffix.
    let url_owned: String;
    let url: &str = match base_url {
        None => OPENAI_DEFAULT_BASE_URL,
        Some(b) if b.contains("/chat/completions") => b,
        Some(b) => {
            let trimmed = b.trim_end_matches('/');
            url_owned = format!("{trimmed}/chat/completions");
            &url_owned
        }
    };
    let body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": user_msg }
        ],
    });
    let resp = client
        .post(url)
        .bearer_auth(api_key)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| CrwError::Internal(format!("LLM request failed: {e}")))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| CrwError::Internal(format!("LLM response read failed: {e}")))?;
    if !status.is_success() {
        return Err(CrwError::Internal(format!("LLM HTTP {status} from openai")));
    }
    let payload: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| CrwError::Internal(format!("LLM response parse failed: {e}")))?;
    let content = payload
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| CrwError::Internal("openai response missing content".to_string()))?;

    let usage = parse_openai_usage(&payload, model, "openai");
    Ok(LlmCallResult {
        content,
        usage,
        warning: None,
    })
}

#[allow(clippy::too_many_arguments)]
async fn call_azure(
    client: &reqwest::Client,
    api_key: &str,
    endpoint: &str,
    deployment: &str,
    api_version: &str,
    max_tokens: u32,
    system_prompt: &str,
    user_msg: &str,
) -> CrwResult<LlmCallResult> {
    let endpoint_trimmed = endpoint.trim_end_matches('/');
    let url = format!(
        "{endpoint_trimmed}/openai/deployments/{deployment}/chat/completions?api-version={api_version}"
    );
    let body = serde_json::json!({
        "max_tokens": max_tokens,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": user_msg }
        ],
    });
    let resp = client
        .post(&url)
        .header("api-key", api_key)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| CrwError::Internal(format!("LLM request failed: {e}")))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| CrwError::Internal(format!("LLM response read failed: {e}")))?;
    if !status.is_success() {
        return Err(CrwError::Internal(format!("LLM HTTP {status} from azure")));
    }
    let payload: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| CrwError::Internal(format!("LLM response parse failed: {e}")))?;
    let content = payload
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| CrwError::Internal("azure response missing content".to_string()))?;

    let usage = parse_openai_usage(&payload, deployment, "azure");
    Ok(LlmCallResult {
        content,
        usage,
        warning: None,
    })
}

fn parse_anthropic_usage(payload: &serde_json::Value, model: &str) -> Option<LlmUsage> {
    let usage = payload.get("usage")?;
    let input_tokens = usage.get("input_tokens").and_then(|v| v.as_u64())? as u32;
    let output_tokens = usage.get("output_tokens").and_then(|v| v.as_u64())? as u32;
    let total = input_tokens + output_tokens;
    Some(LlmUsage {
        input_tokens,
        output_tokens,
        total_tokens: total,
        estimated_cost_usd: pricing::calculate_cost(model, input_tokens, output_tokens),
        model: model.to_string(),
        provider: "anthropic".to_string(),
    })
}

fn parse_openai_usage(
    payload: &serde_json::Value,
    model: &str,
    provider: &str,
) -> Option<LlmUsage> {
    let usage = payload.get("usage")?;
    let input_tokens = usage.get("prompt_tokens").and_then(|v| v.as_u64())? as u32;
    let output_tokens = usage.get("completion_tokens").and_then(|v| v.as_u64())? as u32;
    let total = usage
        .get("total_tokens")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .unwrap_or(input_tokens + output_tokens);
    Some(LlmUsage {
        input_tokens,
        output_tokens,
        total_tokens: total,
        estimated_cost_usd: pricing::calculate_cost(model, input_tokens, output_tokens),
        model: model.to_string(),
        provider: provider.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_api_key_errors_synchronously() {
        let result = extract_via_llm(
            "<html></html>",
            "",
            "anthropic",
            "claude-haiku-4-5",
            None,
            512,
            10_000,
            None,
        )
        .await;
        assert!(matches!(result, Err(CrwError::InvalidRequest(_))));
    }

    #[tokio::test]
    async fn unknown_provider_errors() {
        let result = extract_via_llm(
            "<html></html>",
            "key",
            "groq",
            "model",
            None,
            512,
            10_000,
            None,
        )
        .await;
        assert!(matches!(result, Err(CrwError::InvalidRequest(_))));
    }

    #[tokio::test]
    async fn truncation_respects_char_boundaries() {
        let html = format!("{}🚀tail", "a".repeat(99));
        // Provider unknown so we never make a request, but the truncation
        // logic runs first and would panic on a non-boundary slice.
        let _ = extract_via_llm(&html, "key", "unknown", "m", None, 512, 100, None).await;
    }

    #[tokio::test]
    async fn azure_provider_requires_base_url_and_api_version() {
        let no_base = extract_via_llm(
            "<html></html>",
            "key",
            "azure",
            "gpt-4o-mini",
            None,
            512,
            10_000,
            Some("2024-05-01-preview"),
        )
        .await;
        assert!(matches!(no_base, Err(CrwError::InvalidRequest(_))));
        let no_version = extract_via_llm(
            "<html></html>",
            "key",
            "azure",
            "gpt-4o-mini",
            Some("https://x.openai.azure.com"),
            512,
            10_000,
            None,
        )
        .await;
        assert!(matches!(no_version, Err(CrwError::InvalidRequest(_))));
    }

    #[test]
    fn parse_anthropic_usage_extracts_tokens() {
        let payload = serde_json::json!({
            "usage": { "input_tokens": 100, "output_tokens": 50 }
        });
        let usage = parse_anthropic_usage(&payload, "claude-haiku-4-5").unwrap();
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.total_tokens, 150);
        assert!(usage.estimated_cost_usd.is_some());
    }

    #[test]
    fn parse_openai_usage_extracts_tokens() {
        let payload = serde_json::json!({
            "usage": { "prompt_tokens": 200, "completion_tokens": 100, "total_tokens": 300 }
        });
        let usage = parse_openai_usage(&payload, "gpt-4o-mini", "openai").unwrap();
        assert_eq!(usage.input_tokens, 200);
        assert_eq!(usage.output_tokens, 100);
        assert_eq!(usage.total_tokens, 300);
    }
}
