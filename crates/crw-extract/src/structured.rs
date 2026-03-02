use crw_core::config::LlmConfig;
use crw_core::error::{CrwError, CrwResult};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// Shared HTTP client for LLM API calls (avoids per-request connection overhead).
fn shared_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_default()
    })
}

/// Extract structured JSON from markdown content using an LLM.
pub async fn extract_structured(
    markdown: &str,
    schema: &serde_json::Value,
    llm: &LlmConfig,
) -> CrwResult<serde_json::Value> {
    if llm.api_key.is_empty() {
        return Err(CrwError::ExtractionError(
            "LLM API key is empty. Set [extraction.llm.api_key] or CRW_EXTRACTION__LLM__API_KEY.".into(),
        ));
    }

    let schema_str = serde_json::to_string_pretty(schema)
        .map_err(|e| CrwError::ExtractionError(format!("Invalid JSON schema: {e}")))?;

    let prompt = format!(
        "Extract structured data from the following content according to the JSON schema below.\n\
         Return ONLY valid JSON matching the schema, nothing else. No markdown fences, no explanation.\n\n\
         ## JSON Schema\n```json\n{schema_str}\n```\n\n\
         ## Content\n{markdown}"
    );

    match llm.provider.as_str() {
        "anthropic" => call_anthropic(&prompt, llm).await,
        "openai" => call_openai(&prompt, llm).await,
        other => Err(CrwError::ExtractionError(format!(
            "Unsupported LLM provider: {other}. Use 'anthropic' or 'openai'."
        ))),
    }
}

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<Message>,
}

#[derive(Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<ContentBlock>,
}

#[derive(Deserialize)]
struct ContentBlock {
    text: Option<String>,
}

async fn call_anthropic(prompt: &str, llm: &LlmConfig) -> CrwResult<serde_json::Value> {
    let base_url = llm
        .base_url
        .as_deref()
        .unwrap_or("https://api.anthropic.com");

    let url = format!("{base_url}/v1/messages");

    let body = AnthropicRequest {
        model: llm.model.clone(),
        max_tokens: llm.max_tokens,
        messages: vec![Message {
            role: "user".into(),
            content: prompt.into(),
        }],
    };

    let client = shared_client();
    let resp = client
        .post(&url)
        .header("x-api-key", &llm.api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| CrwError::ExtractionError(format!("Anthropic API request failed: {e}")))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| CrwError::ExtractionError(format!("Failed to read Anthropic response: {e}")))?;

    if !status.is_success() {
        return Err(CrwError::ExtractionError(format!(
            "Anthropic API error ({status}): {}",
            truncate_for_error(&text)
        )));
    }

    let parsed: AnthropicResponse = serde_json::from_str(&text)
        .map_err(|e| CrwError::ExtractionError(format!("Failed to parse Anthropic response: {e}")))?;

    let raw_text = parsed
        .content
        .into_iter()
        .filter_map(|c| c.text)
        .collect::<Vec<_>>()
        .join("");

    parse_json_response(&raw_text)
}

#[derive(Serialize)]
struct OpenAiRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<Message>,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: Message,
}

async fn call_openai(prompt: &str, llm: &LlmConfig) -> CrwResult<serde_json::Value> {
    let base_url = llm
        .base_url
        .as_deref()
        .unwrap_or("https://api.openai.com");

    let url = format!("{base_url}/v1/chat/completions");

    let body = OpenAiRequest {
        model: llm.model.clone(),
        max_tokens: llm.max_tokens,
        messages: vec![Message {
            role: "user".into(),
            content: prompt.into(),
        }],
    };

    let client = shared_client();
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", llm.api_key))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| CrwError::ExtractionError(format!("OpenAI API request failed: {e}")))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| CrwError::ExtractionError(format!("Failed to read OpenAI response: {e}")))?;

    if !status.is_success() {
        return Err(CrwError::ExtractionError(format!(
            "OpenAI API error ({status}): {}",
            truncate_for_error(&text)
        )));
    }

    let parsed: OpenAiResponse = serde_json::from_str(&text)
        .map_err(|e| CrwError::ExtractionError(format!("Failed to parse OpenAI response: {e}")))?;

    let raw_text = parsed
        .choices
        .first()
        .map(|c| c.message.content.clone())
        .unwrap_or_default();

    parse_json_response(&raw_text)
}

/// Parse JSON from LLM response, stripping markdown fences if present.
fn parse_json_response(text: &str) -> CrwResult<serde_json::Value> {
    let trimmed = text.trim();

    // Strip ```json ... ``` fences if LLM wrapped it
    let json_str = if trimmed.starts_with("```") {
        let inner = trimmed
            .strip_prefix("```json")
            .or_else(|| trimmed.strip_prefix("```"))
            .unwrap_or(trimmed);
        inner
            .strip_suffix("```")
            .unwrap_or(inner)
            .trim()
    } else {
        trimmed
    };

    serde_json::from_str(json_str).map_err(|e| {
        CrwError::ExtractionError(format!(
            "LLM returned invalid JSON: {e}\nResponse preview: {}",
            truncate_for_error(text)
        ))
    })
}

/// Truncate text for error messages to avoid leaking large responses.
fn truncate_for_error(text: &str) -> &str {
    if text.len() > 200 {
        &text[..200]
    } else {
        text
    }
}
