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

/// Validate a JSON value against a JSON schema.
fn validate_against_schema(
    value: &serde_json::Value,
    schema: &serde_json::Value,
) -> CrwResult<()> {
    let validator = jsonschema::validator_for(schema)
        .map_err(|e| CrwError::ExtractionError(format!("Invalid JSON schema: {e}")))?;
    let errors: Vec<String> = validator.iter_errors(value).map(|e| e.to_string()).collect();
    if !errors.is_empty() {
        return Err(CrwError::ExtractionError(format!(
            "LLM output failed schema validation:\n{}",
            errors.join("\n")
        )));
    }
    Ok(())
}

/// Extract structured JSON from markdown content using an LLM.
pub async fn extract_structured(
    markdown: &str,
    schema: &serde_json::Value,
    llm: &LlmConfig,
) -> CrwResult<serde_json::Value> {
    if llm.api_key.is_empty() {
        return Err(CrwError::ExtractionError(
            "LLM API key is empty. Set [extraction.llm.api_key] or CRW_EXTRACTION__LLM__API_KEY."
                .into(),
        ));
    }

    let result = match llm.provider.as_str() {
        "anthropic" => call_anthropic(markdown, schema, llm).await,
        "openai" => call_openai(markdown, schema, llm).await,
        other => Err(CrwError::ExtractionError(format!(
            "Unsupported LLM provider: {other}. Use 'anthropic' or 'openai'."
        ))),
    }?;

    validate_against_schema(&result, schema)?;
    Ok(result)
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

async fn call_anthropic(
    markdown: &str,
    schema: &serde_json::Value,
    llm: &LlmConfig,
) -> CrwResult<serde_json::Value> {
    let base_url = llm
        .base_url
        .as_deref()
        .unwrap_or("https://api.anthropic.com");

    let url = format!("{base_url}/v1/messages");

    let prompt = format!(
        "Extract structured data from the following content according to the JSON schema. \
         Call the extract_data tool with the extracted data.\n\n## Content\n{markdown}"
    );

    let body = AnthropicRequest {
        model: llm.model.clone(),
        max_tokens: llm.max_tokens,
        messages: vec![Message {
            role: "user".into(),
            content: prompt,
        }],
        tools: Some(vec![AnthropicTool {
            name: "extract_data".into(),
            description: "Extract structured data from the content".into(),
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

    // Try tool_use blocks first (structured output).
    for block in &parsed.content {
        if let AnthropicContentBlock::ToolUse { input, .. } = block {
            return Ok(input.clone());
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

    parse_json_response(&raw_text)
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

async fn call_openai(
    markdown: &str,
    schema: &serde_json::Value,
    llm: &LlmConfig,
) -> CrwResult<serde_json::Value> {
    let base_url = llm
        .base_url
        .as_deref()
        .unwrap_or("https://api.openai.com");

    let url = format!("{base_url}/v1/chat/completions");

    let prompt = format!(
        "Extract structured data from the following content according to the provided schema. \
         Call the extract_data function with the extracted data.\n\n## Content\n{markdown}"
    );

    let body = OpenAiRequest {
        model: llm.model.clone(),
        max_tokens: llm.max_tokens,
        messages: vec![Message {
            role: "user".into(),
            content: prompt,
        }],
        tools: Some(vec![OpenAiToolDef {
            r#type: "function".into(),
            function: OpenAiFunctionDef {
                name: "extract_data".into(),
                description: "Extract structured data from the content".into(),
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

    let choice = parsed
        .choices
        .first()
        .ok_or_else(|| CrwError::ExtractionError("OpenAI returned no choices".into()))?;

    // Try tool_calls first (function calling).
    if let Some(tool_calls) = &choice.message.tool_calls {
        if let Some(call) = tool_calls.first() {
            return serde_json::from_str(&call.function.arguments).map_err(|e| {
                CrwError::ExtractionError(format!(
                    "Failed to parse OpenAI function call arguments: {e}"
                ))
            });
        }
    }

    // Fallback: extract from content text.
    let raw_text = choice.message.content.clone().unwrap_or_default();
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
}
