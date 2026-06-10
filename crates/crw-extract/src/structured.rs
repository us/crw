use crate::pricing;
use crw_core::config::LlmConfig;
use crw_core::error::{CrwError, CrwResult};
use crw_core::types::LlmUsage;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// Request timeout for LLM API calls.
const LLM_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Default UTF-8-safe truncation ceiling on markdown sent to the LLM for
/// structured extraction. Matches the Next.js side's pre-flight cap so the
/// per-call reserve never goes wildly out of band. Pages larger than this
/// can still be processed with an explicit caller-supplied override.
pub const DEFAULT_MAX_INPUT_BYTES: usize = 50_000;

/// Result of a structured-extraction LLM call: the validated JSON value
/// plus per-call token usage and a `truncated` flag indicating whether the
/// markdown input was clipped at [`DEFAULT_MAX_INPUT_BYTES`] (or the
/// caller-supplied override) before being sent to the LLM.
#[derive(Debug, Clone)]
pub struct StructuredExtractResult {
    pub value: serde_json::Value,
    pub usage: Option<LlmUsage>,
    pub truncated: bool,
}

/// UTF-8-safe truncation: clip at `max_bytes` but walk back to the nearest
/// char boundary so we never split a multibyte sequence. Returns
/// `(truncated_slice, was_truncated)`.
pub(crate) fn truncate_md(s: &str, max_bytes: usize) -> (&str, bool) {
    if s.len() <= max_bytes {
        return (s, false);
    }
    let mut idx = max_bytes;
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    (&s[..idx], true)
}

/// Shared HTTP client for LLM API calls (avoids per-request connection overhead).
fn shared_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(LLM_REQUEST_TIMEOUT)
            .build()
            .unwrap_or_default()
    })
}

/// Validate a JSON value against a JSON schema.
pub(crate) fn validate_against_schema(
    value: &serde_json::Value,
    schema: &serde_json::Value,
) -> CrwResult<()> {
    let validator = jsonschema::validator_for(schema)
        .map_err(|e| CrwError::ExtractionError(format!("Invalid JSON schema: {e}")))?;
    let errors: Vec<String> = validator
        .iter_errors(value)
        .map(|e| e.to_string())
        .collect();
    if !errors.is_empty() {
        return Err(CrwError::ExtractionError(format!(
            "LLM output failed schema validation:\n{}",
            errors.join("\n")
        )));
    }
    Ok(())
}

/// Extract structured JSON from markdown content using an LLM.
///
/// Backward-compatible thin wrapper: callers that only need the validated
/// JSON value can keep calling this. New callers that also want the LLM
/// token-usage envelope + truncation flag should use
/// [`extract_structured_with_usage`].
pub async fn extract_structured(
    markdown: &str,
    schema: &serde_json::Value,
    llm: &LlmConfig,
) -> CrwResult<serde_json::Value> {
    Ok(extract_structured_with_usage(markdown, schema, llm, None)
        .await?
        .value)
}

/// Extract structured JSON and return token usage + truncation status.
///
/// `max_input_bytes` overrides the per-call markdown byte ceiling. `None`
/// falls back to [`DEFAULT_MAX_INPUT_BYTES`] (50 KB). Truncation is done
/// on a UTF-8 char boundary; if it occurred, the returned
/// [`StructuredExtractResult::truncated`] is `true` and the
/// `LlmUsage.truncated` field (when usage is present) is also set so
/// downstream billing surfaces can flag pages that were clipped.
pub async fn extract_structured_with_usage(
    markdown: &str,
    schema: &serde_json::Value,
    llm: &LlmConfig,
    max_input_bytes: Option<usize>,
) -> CrwResult<StructuredExtractResult> {
    if llm.api_key.is_empty() {
        return Err(CrwError::ExtractionError(
            "LLM API key is empty. Set [extraction.llm.api_key] or CRW_EXTRACTION__LLM__API_KEY."
                .into(),
        ));
    }

    let max_bytes = max_input_bytes.unwrap_or(DEFAULT_MAX_INPUT_BYTES);
    let (clipped, truncated) = truncate_md(markdown, max_bytes);

    let prompt = format!(
        "Extract structured data from the following content according to the JSON schema. \
         Call the extract_data tool with the extracted data.\n\n## Content\n{clipped}"
    );

    let (value, mut usage) = match llm.provider.as_str() {
        "anthropic" => {
            call_anthropic(
                &prompt,
                schema,
                llm,
                "extract_data",
                "Extract structured data from the content",
            )
            .await
        }
        "openai" | "deepseek" | "openai-compatible" => {
            call_openai(
                &prompt,
                schema,
                llm,
                "extract_data",
                "Extract structured data from the content",
            )
            .await
        }
        other => Err(CrwError::ExtractionError(format!(
            "Unsupported LLM provider: {other}. Use 'anthropic', 'openai', 'deepseek', or 'openai-compatible'."
        ))),
    }?;

    if truncated && let Some(u) = usage.as_mut() {
        u.truncated = true;
    }

    validate_against_schema(&value, schema)?;
    Ok(StructuredExtractResult {
        value,
        usage,
        truncated,
    })
}

// ── Anthropic ──

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
}

#[derive(Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContentBlock>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Deserialize, Default)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: Option<u32>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u32>,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum AnthropicContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        #[allow(dead_code)]
        id: String,
        #[allow(dead_code)]
        name: String,
        input: serde_json::Value,
    },
}

/// Call Anthropic with a tool-use forcing the given `schema`. `prompt` is the
/// full user message; `tool_name`/`tool_desc` name the forced tool. Shared by
/// structured extraction and the change-tracking judge.
pub(crate) async fn call_anthropic(
    prompt: &str,
    schema: &serde_json::Value,
    llm: &LlmConfig,
    tool_name: &str,
    tool_desc: &str,
) -> CrwResult<(serde_json::Value, Option<LlmUsage>)> {
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
            content: prompt.to_string(),
        }],
        tools: Some(vec![AnthropicTool {
            name: tool_name.into(),
            description: tool_desc.into(),
            input_schema: schema.clone(),
        }]),
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
    let text = resp.text().await.map_err(|e| {
        CrwError::ExtractionError(format!("Failed to read Anthropic response: {e}"))
    })?;

    if !status.is_success() {
        return Err(CrwError::ExtractionError(format!(
            "Anthropic API error ({status}): {}",
            truncate_for_error(&text)
        )));
    }

    let parsed: AnthropicResponse = serde_json::from_str(&text).map_err(|e| {
        CrwError::ExtractionError(format!("Failed to parse Anthropic response: {e}"))
    })?;

    let usage = parsed.usage.as_ref().map(|u| {
        let (cache_hit, cache_miss) =
            match (u.cache_read_input_tokens, u.cache_creation_input_tokens) {
                (None, None) => (None, None),
                (read, create) => {
                    let hit = read.unwrap_or(0);
                    let create = create.unwrap_or(0);
                    let miss = u.input_tokens.saturating_add(create);
                    (Some(hit), Some(miss))
                }
            };
        LlmUsage {
            input_tokens: u.input_tokens,
            output_tokens: u.output_tokens,
            total_tokens: u.input_tokens + u.output_tokens,
            estimated_cost_usd: pricing::calculate_cost(
                &llm.model,
                u.input_tokens,
                u.output_tokens,
            ),
            model: llm.model.clone(),
            provider: "anthropic".to_string(),
            cache_hit_input_tokens: cache_hit,
            cache_miss_input_tokens: cache_miss,
            truncated: false,
            calls: 1,
            // R1 counters are aggregated in the /v1/search caller;
            // single-call sites always emit defaults.
            executed_summaries: 0,
            answer_executed: false,
        }
    });

    // Try tool_use blocks first (structured output).
    for block in &parsed.content {
        if let AnthropicContentBlock::ToolUse { input, .. } = block {
            return Ok((input.clone(), usage));
        }
    }

    // Fallback: extract from text blocks.
    let raw_text: String = parsed
        .content
        .into_iter()
        .filter_map(|c| match c {
            AnthropicContentBlock::Text { text } => Some(text),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");

    let value = parse_json_response(&raw_text)?;
    Ok((value, usage))
}

// ── OpenAI ──

#[derive(Serialize)]
struct OpenAiRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAiToolDef>>,
}

#[derive(Serialize)]
struct OpenAiToolDef {
    r#type: String,
    function: OpenAiFunctionDef,
}

#[derive(Serialize)]
struct OpenAiFunctionDef {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Deserialize, Default)]
struct OpenAiUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    total_tokens: Option<u32>,
    #[serde(default)]
    prompt_cache_hit_tokens: Option<u32>,
    #[serde(default)]
    prompt_cache_miss_tokens: Option<u32>,
    #[serde(default)]
    prompt_tokens_details: Option<OpenAiPromptDetails>,
}

#[derive(Deserialize, Default)]
struct OpenAiPromptDetails {
    #[serde(default)]
    cached_tokens: Option<u32>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
}

#[derive(Deserialize)]
struct OpenAiMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Deserialize)]
struct OpenAiToolCall {
    function: OpenAiFunctionCall,
}

#[derive(Deserialize)]
struct OpenAiFunctionCall {
    #[allow(dead_code)]
    name: String,
    arguments: String,
}

/// Resolve the chat-completions endpoint for an OpenAI-compatible provider.
///
/// Idempotent: a `base_url` that already points at `…/chat/completions` is used
/// verbatim; a bare base (or the provider default) gets `/v1/chat/completions`
/// appended. This mirrors `llm::call_openai` so a configured base_url such as
/// `https://api.deepseek.com/v1/chat/completions` is not doubled into
/// `…/v1/chat/completions/v1/chat/completions` (which 404s).
fn openai_chat_url(base_url: Option<&str>, default_base: &str) -> String {
    match base_url {
        Some(b) if b.contains("/chat/completions") => b.to_string(),
        Some(b) => format!("{}/v1/chat/completions", b.trim_end_matches('/')),
        None => format!("{}/v1/chat/completions", default_base.trim_end_matches('/')),
    }
}

/// Call an OpenAI-compatible provider with a function-call forcing the given
/// `schema`. `prompt` is the full user message; `tool_name`/`tool_desc` name
/// the forced function. Shared by structured extraction and the judge.
pub(crate) async fn call_openai(
    prompt: &str,
    schema: &serde_json::Value,
    llm: &LlmConfig,
    tool_name: &str,
    tool_desc: &str,
) -> CrwResult<(serde_json::Value, Option<LlmUsage>)> {
    let default_base = match llm.provider.as_str() {
        "deepseek" => "https://api.deepseek.com",
        _ => "https://api.openai.com",
    };
    let url = openai_chat_url(llm.base_url.as_deref(), default_base);

    let body = OpenAiRequest {
        model: llm.model.clone(),
        max_tokens: llm.max_tokens,
        messages: vec![Message {
            role: "user".into(),
            content: prompt.to_string(),
        }],
        tools: Some(vec![OpenAiToolDef {
            r#type: "function".into(),
            function: OpenAiFunctionDef {
                name: tool_name.into(),
                description: tool_desc.into(),
                parameters: schema.clone(),
            },
        }]),
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

    let usage = parsed.usage.as_ref().map(|u| {
        let total = u
            .total_tokens
            .unwrap_or_else(|| u.prompt_tokens + u.completion_tokens);
        let openai_cached = u
            .prompt_tokens_details
            .as_ref()
            .and_then(|d| d.cached_tokens);
        let (cache_hit, cache_miss) = match (
            u.prompt_cache_hit_tokens,
            u.prompt_cache_miss_tokens,
            openai_cached,
        ) {
            (Some(h), Some(m), _) => (Some(h), Some(m)),
            (Some(h), None, _) => (Some(h), Some(u.prompt_tokens.saturating_sub(h))),
            (None, Some(m), _) => (Some(u.prompt_tokens.saturating_sub(m)), Some(m)),
            (None, None, Some(c)) => (Some(c), Some(u.prompt_tokens.saturating_sub(c))),
            (None, None, None) => (None, None),
        };
        LlmUsage {
            input_tokens: u.prompt_tokens,
            output_tokens: u.completion_tokens,
            total_tokens: total,
            estimated_cost_usd: pricing::calculate_cost(
                &llm.model,
                u.prompt_tokens,
                u.completion_tokens,
            ),
            model: llm.model.clone(),
            // NOTE: structured.rs is reached only when the dispatcher in
            // extract_structured() matched "openai". DeepSeek goes through
            // the lib.rs/llm.rs path and is tagged correctly there. If a
            // future caller routes DeepSeek through this file, this tag
            // must thread through too.
            provider: llm.provider.clone(),
            cache_hit_input_tokens: cache_hit,
            cache_miss_input_tokens: cache_miss,
            truncated: false,
            calls: 1,
            // R1 counters are aggregated in the /v1/search caller;
            // single-call sites always emit defaults.
            executed_summaries: 0,
            answer_executed: false,
        }
    });

    let choice = parsed
        .choices
        .first()
        .ok_or_else(|| CrwError::ExtractionError("OpenAI returned no choices".into()))?;

    // Try tool_calls first (function calling).
    if let Some(tool_calls) = &choice.message.tool_calls
        && let Some(call) = tool_calls.first()
    {
        let value: serde_json::Value =
            serde_json::from_str(&call.function.arguments).map_err(|e| {
                CrwError::ExtractionError(format!(
                    "Failed to parse OpenAI function call arguments: {e}"
                ))
            })?;
        return Ok((value, usage));
    }

    // Fallback: extract from content text.
    let raw_text = choice.message.content.clone().unwrap_or_default();
    let value = parse_json_response(&raw_text)?;
    Ok((value, usage))
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
        inner.strip_suffix("```").unwrap_or(inner).trim()
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
        &text[..text.floor_char_boundary(200)]
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_validate_against_schema_success() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "integer" }
            },
            "required": ["name"]
        });
        let value = json!({ "name": "Alice", "age": 30 });
        assert!(validate_against_schema(&value, &schema).is_ok());
    }

    #[test]
    fn test_validate_against_schema_missing_required() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "integer" }
            },
            "required": ["name"]
        });
        let value = json!({ "age": 30 });
        let err = validate_against_schema(&value, &schema).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("schema validation"), "got: {msg}");
    }

    #[test]
    fn test_validate_against_schema_wrong_type() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            },
            "required": ["name"]
        });
        let value = json!({ "name": 123 });
        let err = validate_against_schema(&value, &schema).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("schema validation"), "got: {msg}");
    }

    #[test]
    fn test_parse_json_response_plain() {
        let result = parse_json_response(r#"{"key": "value"}"#).unwrap();
        assert_eq!(result, json!({"key": "value"}));
    }

    #[test]
    fn test_parse_json_response_with_fences() {
        let result = parse_json_response("```json\n{\"key\": \"value\"}\n```").unwrap();
        assert_eq!(result, json!({"key": "value"}));
    }

    #[test]
    fn truncate_md_passes_through_short_input() {
        let s = "hello world";
        let (out, was) = truncate_md(s, 50_000);
        assert_eq!(out, s);
        assert!(!was);
    }

    #[test]
    fn truncate_md_clips_at_default_50k_byte_cutoff() {
        // Build a payload larger than DEFAULT_MAX_INPUT_BYTES (50_000) where
        // a multibyte char STRADDLES the 50_000-byte boundary. The 4-byte
        // rocket emoji at byte 49_998 occupies bytes 49_998..=50_001; a
        // naive slice at 50_000 would split it and panic. The safe
        // truncation must walk back to byte 49_998.
        let prefix = "a".repeat(49_998);
        let big = format!("{prefix}🚀{}", "z".repeat(10_000));
        assert!(big.len() > DEFAULT_MAX_INPUT_BYTES);
        let (out, was) = truncate_md(&big, DEFAULT_MAX_INPUT_BYTES);
        assert!(was, "expected truncation to fire above 50 KB");
        assert!(
            out.is_char_boundary(out.len()),
            "truncated slice must end on a UTF-8 char boundary"
        );
        // The walked-back boundary lands before the emoji, NOT mid-emoji.
        assert_eq!(out.len(), 49_998);
        // And the prefix is intact — every byte is 'a'.
        assert!(out.bytes().all(|b| b == b'a'));
    }

    #[test]
    fn truncate_md_honours_explicit_smaller_cap() {
        let s = format!("{}🚀tail", "a".repeat(99));
        let (out, was) = truncate_md(&s, 100);
        assert!(was);
        // 99 'a's fit; emoji starts at byte 99 (4 bytes) — must NOT split.
        assert!(out.len() <= 100);
        assert!(out.is_char_boundary(out.len()));
    }

    #[test]
    fn openai_url_appends_path_to_bare_base() {
        assert_eq!(
            openai_chat_url(Some("https://api.deepseek.com"), "https://api.openai.com"),
            "https://api.deepseek.com/v1/chat/completions"
        );
    }

    #[test]
    fn openai_url_uses_full_endpoint_verbatim() {
        // Regression: a base_url that already includes the path must NOT be
        // doubled into `…/v1/chat/completions/v1/chat/completions` (→ 404).
        let full = "https://api.deepseek.com/v1/chat/completions";
        assert_eq!(openai_chat_url(Some(full), "https://api.openai.com"), full);
    }

    #[test]
    fn openai_url_falls_back_to_default_base() {
        assert_eq!(
            openai_chat_url(None, "https://api.openai.com"),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn openai_url_trims_trailing_slash() {
        assert_eq!(
            openai_chat_url(Some("https://api.deepseek.com/"), "https://api.openai.com"),
            "https://api.deepseek.com/v1/chat/completions"
        );
    }
}
