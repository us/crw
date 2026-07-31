//! OpenAI Responses API transport shared by text generation and structured
//! extraction.
//!
//! The Responses wire format is not Chat Completions with a different path:
//! function tools are flat objects, generated calls live in `output`, and token
//! usage uses `input_tokens` / `output_tokens`. Keep that translation isolated
//! here so OpenAI-compatible Chat Completions providers remain unchanged.

use crate::llm::LlmCallResult;
use crate::pricing;
use crw_core::config::LlmConfig;
use crw_core::error::{CrwError, CrwResult};
use crw_core::types::LlmUsage;
use std::sync::OnceLock;
use std::time::Duration;

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

fn shared_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .expect("reqwest client build (Responses shared)")
    })
}

/// Resolve a Responses endpoint from either a versioned API base or a complete
/// endpoint. A base such as `https://gateway.example/v1` therefore becomes
/// `https://gateway.example/v1/responses`, while a complete endpoint is idempotent.
pub(crate) fn responses_url(base_url: Option<&str>) -> String {
    let base = base_url.unwrap_or(DEFAULT_BASE_URL).trim_end_matches('/');
    if base.ends_with("/responses") {
        base.to_string()
    } else {
        format!("{base}/responses")
    }
}

fn apply_optional_generation_fields(body: &mut serde_json::Value, llm: &LlmConfig) {
    if let Some(temperature) = llm.temperature {
        body["temperature"] = serde_json::json!(temperature);
    }
    if let Some(effort) = llm.reasoning_effort.as_deref().filter(|s| !s.is_empty()) {
        body["reasoning"] = serde_json::json!({ "effort": effort });
    }
}

async fn post(
    llm: &LlmConfig,
    body: &serde_json::Value,
    timeout: Duration,
) -> CrwResult<serde_json::Value> {
    let url = responses_url(llm.base_url.as_deref());
    let resp = shared_client()
        .post(&url)
        .timeout(timeout)
        .bearer_auth(&llm.api_key)
        .header("content-type", "application/json")
        .json(body)
        .send()
        .await
        .map_err(|e| CrwError::ExtractionError(format!("Responses API request failed: {e}")))?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| {
        CrwError::ExtractionError(format!("Failed to read Responses API response: {e}"))
    })?;
    if !status.is_success() {
        return Err(CrwError::ExtractionError(format!(
            "Responses API error ({status}): {}",
            truncate_for_error(&text)
        )));
    }

    serde_json::from_str(&text).map_err(|e| {
        CrwError::ExtractionError(format!("Failed to parse Responses API response: {e}"))
    })
}

fn output_text(payload: &serde_json::Value) -> String {
    payload
        .get("output")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(serde_json::Value::as_str) == Some("message"))
        .filter_map(|item| item.get("content").and_then(serde_json::Value::as_array))
        .flatten()
        .filter(|part| part.get("type").and_then(serde_json::Value::as_str) == Some("output_text"))
        .filter_map(|part| part.get("text").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>()
        .join("")
}

fn parse_usage(payload: &serde_json::Value, llm: &LlmConfig) -> Option<LlmUsage> {
    let usage = payload.get("usage")?;
    let input_tokens = usage.get("input_tokens")?.as_u64()? as u32;
    let output_tokens = usage.get("output_tokens")?.as_u64()? as u32;
    let total_tokens = usage
        .get("total_tokens")
        .and_then(serde_json::Value::as_u64)
        .map(|n| n as u32)
        .unwrap_or(input_tokens.saturating_add(output_tokens));
    let cached = usage
        .get("input_tokens_details")
        .and_then(|details| details.get("cached_tokens"))
        .and_then(serde_json::Value::as_u64)
        .map(|n| n as u32);

    Some(LlmUsage {
        input_tokens,
        output_tokens,
        total_tokens,
        estimated_cost_usd: pricing::calculate_cost(&llm.model, input_tokens, output_tokens),
        model: llm.model.clone(),
        provider: "openai-responses".to_string(),
        cache_hit_input_tokens: cached,
        cache_miss_input_tokens: cached.map(|n| input_tokens.saturating_sub(n)),
        truncated: false,
        calls: 1,
        executed_summaries: 0,
        answer_executed: false,
    })
}

/// One non-streaming text response. `instructions` carries the trusted system
/// prompt and `input` carries the user/content message.
pub(crate) async fn call_text(
    llm: &LlmConfig,
    instructions: &str,
    input: &str,
    timeout: Duration,
) -> CrwResult<LlmCallResult> {
    let mut body = serde_json::json!({
        "model": llm.model,
        "instructions": instructions,
        "input": input,
        "max_output_tokens": llm.max_tokens,
        "store": false,
    });
    apply_optional_generation_fields(&mut body, llm);

    let payload = post(llm, &body, timeout).await?;
    let content = output_text(&payload);
    if content.is_empty() {
        return Err(CrwError::ExtractionError(
            "Responses API response missing output_text".into(),
        ));
    }
    Ok(LlmCallResult {
        content,
        usage: parse_usage(&payload, llm),
        warning: None,
    })
}

/// One forced function call used as the structured-output envelope. CRW does
/// not execute the function; its JSON arguments are the extraction result.
pub(crate) async fn call_tool(
    llm: &LlmConfig,
    input: &str,
    schema: &serde_json::Value,
    tool_name: &str,
    tool_desc: &str,
    timeout: Duration,
) -> CrwResult<(serde_json::Value, Option<LlmUsage>)> {
    let mut body = serde_json::json!({
        "model": llm.model,
        "input": input,
        "max_output_tokens": llm.max_tokens,
        "store": false,
        "parallel_tool_calls": false,
        "tools": [{
            "type": "function",
            "name": tool_name,
            "description": tool_desc,
            "parameters": schema,
        }],
        "tool_choice": {
            "type": "function",
            "name": tool_name,
        },
    });
    apply_optional_generation_fields(&mut body, llm);

    let payload = post(llm, &body, timeout).await?;
    let usage = parse_usage(&payload, llm);
    if let Some(arguments) = payload
        .get("output")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .find(|item| {
            item.get("type").and_then(serde_json::Value::as_str) == Some("function_call")
                && item.get("name").and_then(serde_json::Value::as_str) == Some(tool_name)
        })
        .and_then(|item| item.get("arguments"))
        .and_then(serde_json::Value::as_str)
    {
        let value = serde_json::from_str(arguments).map_err(|e| {
            CrwError::ExtractionError(format!(
                "Failed to parse Responses function call arguments: {e}"
            ))
        })?;
        return Ok((value, usage));
    }

    // Some compatibility gateways ignore tool_choice and return JSON text.
    // Preserve CRW's existing fallback behavior, then apply normal schema
    // validation in the caller.
    let raw_text = output_text(&payload);
    let value = crate::structured::parse_json_response(&raw_text)?;
    Ok((value, usage))
}

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

    #[test]
    fn endpoint_appends_responses_to_versioned_base() {
        assert_eq!(
            responses_url(Some("https://gateway.example/v1")),
            "https://gateway.example/v1/responses"
        );
    }

    #[test]
    fn endpoint_preserves_complete_url() {
        assert_eq!(
            responses_url(Some("https://example.test/v1/responses/")),
            "https://example.test/v1/responses"
        );
    }
}
