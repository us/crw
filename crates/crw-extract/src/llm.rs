//! LLM-assisted content extraction fallback.
//!
//! Used when DOM-based extraction (readability + heuristics) yields a
//! markdown candidate whose quality score is below the configured threshold.
//! The raw HTML (truncated to a byte cap) is sent to a chat-completions API;
//! the response is treated as a 6th candidate and run through the same
//! quality scorer as the rest.
//!
//! Supports two providers: `anthropic` (default) and `openai`.

use crw_core::error::{CrwError, CrwResult};
use std::time::Duration;

const ANTHROPIC_DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1/messages";
const OPENAI_DEFAULT_BASE_URL: &str = "https://api.openai.com/v1/chat/completions";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

const SYSTEM_PROMPT: &str = "You extract the main article or content body from a web page's HTML. \
Return only the article text as plain markdown. Strip nav, header, footer, ads, comments, related links, \
sidebars, cookie banners, share buttons, author bios, social widgets. No preamble or commentary, no fenced \
code block — just the markdown content.";

/// Call the configured LLM to extract main content from raw HTML.
///
/// `html` is truncated to `max_html_bytes` before sending. The body is
/// returned as a markdown string. Errors map to `CrwError::Internal` so the
/// caller can choose to drop the fallback silently and keep the original.
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
    let truncated = if html.len() > max_html_bytes {
        // Slice on a UTF-8 char boundary just below the limit.
        let mut idx = max_html_bytes;
        while idx > 0 && !html.is_char_boundary(idx) {
            idx -= 1;
        }
        &html[..idx]
    } else {
        html
    };
    let user_msg =
        format!("Extract the main article/content body from this HTML as markdown:\n\n{truncated}");

    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| CrwError::Internal(format!("LLM client build failed: {e}")))?;

    match provider.to_ascii_lowercase().as_str() {
        "anthropic" => {
            call_anthropic(&client, api_key, model, base_url, max_tokens, &user_msg).await
        }
        "openai" => call_openai(&client, api_key, model, base_url, max_tokens, &user_msg).await,
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
                &client, api_key, endpoint, model, version, max_tokens, &user_msg,
            )
            .await
        }
        other => Err(CrwError::InvalidRequest(format!(
            "unknown LLM provider: {other}"
        ))),
    }
}

async fn call_azure(
    client: &reqwest::Client,
    api_key: &str,
    endpoint: &str,
    deployment: &str,
    api_version: &str,
    max_tokens: u32,
    user_msg: &str,
) -> CrwResult<String> {
    // Azure OpenAI URL shape:
    //   {endpoint}/openai/deployments/{deployment}/chat/completions?api-version={version}
    // Body is OpenAI-compatible chat.completions; auth is `api-key` header,
    // not bearer.
    let endpoint_trimmed = endpoint.trim_end_matches('/');
    let url = format!(
        "{endpoint_trimmed}/openai/deployments/{deployment}/chat/completions?api-version={api_version}"
    );
    let body = serde_json::json!({
        "max_tokens": max_tokens,
        "messages": [
            { "role": "system", "content": SYSTEM_PROMPT },
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
    let payload: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| CrwError::Internal(format!("LLM response parse failed: {e}")))?;
    if !status.is_success() {
        return Err(CrwError::Internal(format!("LLM HTTP {status}: {payload}")));
    }
    payload
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| CrwError::Internal(format!("LLM response missing content: {payload}")))
}

async fn call_anthropic(
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    base_url: Option<&str>,
    max_tokens: u32,
    user_msg: &str,
) -> CrwResult<String> {
    let url = base_url.unwrap_or(ANTHROPIC_DEFAULT_BASE_URL);
    let body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "system": SYSTEM_PROMPT,
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
    let payload: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| CrwError::Internal(format!("LLM response parse failed: {e}")))?;
    if !status.is_success() {
        return Err(CrwError::Internal(format!("LLM HTTP {status}: {payload}")));
    }
    payload
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| {
            arr.iter()
                .find_map(|b| b.get("text").and_then(|t| t.as_str()))
        })
        .map(|s| s.to_string())
        .ok_or_else(|| CrwError::Internal(format!("LLM response missing text: {payload}")))
}

async fn call_openai(
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    base_url: Option<&str>,
    max_tokens: u32,
    user_msg: &str,
) -> CrwResult<String> {
    let url = base_url.unwrap_or(OPENAI_DEFAULT_BASE_URL);
    let body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": [
            { "role": "system", "content": SYSTEM_PROMPT },
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
    let payload: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| CrwError::Internal(format!("LLM response parse failed: {e}")))?;
    if !status.is_success() {
        return Err(CrwError::Internal(format!("LLM HTTP {status}: {payload}")));
    }
    payload
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| CrwError::Internal(format!("LLM response missing content: {payload}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_api_key_errors_synchronously() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(extract_via_llm(
            "<html></html>",
            "",
            "anthropic",
            "claude-haiku-4-5",
            None,
            512,
            10_000,
            None,
        ));
        assert!(matches!(result, Err(CrwError::InvalidRequest(_))));
    }

    #[test]
    fn unknown_provider_errors() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(extract_via_llm(
            "<html></html>",
            "key",
            "groq",
            "model",
            None,
            512,
            10_000,
            None,
        ));
        assert!(matches!(result, Err(CrwError::InvalidRequest(_))));
    }

    #[test]
    fn truncation_respects_char_boundaries() {
        // 4-byte char (rocket emoji) at the boundary — must not be split.
        let html = format!("{}🚀tail", "a".repeat(99));
        // max_bytes = 100 splits the emoji at byte 99; we should fall back
        // to byte 99 rather than panic.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        // Provider unknown so we never make a request, but the truncation
        // logic runs first and would panic on a non-boundary slice.
        let _ = rt.block_on(extract_via_llm(
            &html, "key", "unknown", "m", None, 512, 100, None,
        ));
    }

    #[test]
    fn azure_provider_requires_base_url_and_api_version() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let no_base = rt.block_on(extract_via_llm(
            "<html></html>",
            "key",
            "azure",
            "gpt-4o-mini",
            None,
            512,
            10_000,
            Some("2024-05-01-preview"),
        ));
        assert!(matches!(no_base, Err(CrwError::InvalidRequest(_))));
        let no_version = rt.block_on(extract_via_llm(
            "<html></html>",
            "key",
            "azure",
            "gpt-4o-mini",
            Some("https://x.openai.azure.com"),
            512,
            10_000,
            None,
        ));
        assert!(matches!(no_version, Err(CrwError::InvalidRequest(_))));
    }
}
