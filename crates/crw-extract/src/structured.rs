use crate::basis;
use crate::pricing;
use crw_core::config::LlmConfig;
use crw_core::error::{CrwError, CrwResult};
use crw_core::evidence::{Basis, BasisWarning};
use crw_core::types::LlmUsage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;
use std::time::Duration;

/// Request timeout for LLM API calls.
pub(crate) const LLM_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Request timeout for a basis extraction. Basis rides the same call but grows
/// its **output** by 2-4k tokens, and output tokens are the serial-decode term:
/// at the slower providers' 15-30 tok/s that alone exceeds the 60s default.
/// Applied per-request, so the judge and summary paths keep the 60s bound.
const BASIS_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

/// Default UTF-8-safe truncation ceiling on markdown sent to the LLM for
/// structured extraction. Matches the Next.js side's pre-flight cap so the
/// per-call reserve never goes wildly out of band. Pages larger than this
/// can still be processed with an explicit caller-supplied override.
pub const DEFAULT_MAX_INPUT_BYTES: usize = 50_000;

/// Result of a structured-extraction LLM call: the validated JSON value
/// plus per-call token usage and a `truncated` flag indicating whether the
/// markdown input was clipped at [`DEFAULT_MAX_INPUT_BYTES`] (or the
/// caller-supplied override) before being sent to the LLM.
///
/// The `basis*` / `llm_input_hash` fields are populated only by
/// [`extract_structured_with_basis`]; they stay empty on the plain path.
#[derive(Debug, Clone, Default)]
pub struct StructuredExtractResult {
    pub value: serde_json::Value,
    pub usage: Option<LlmUsage>,
    pub truncated: bool,
    /// Per-field evidence, one entry per top-level scalar schema property.
    pub basis: Vec<Basis>,
    /// Coded explanations for every basis downgrade. Never upstream text.
    pub basis_warnings: Vec<BasisWarning>,
    /// `"sha256:"`-prefixed hash of the canonical source text — the exact
    /// (truncated) markdown sent to the model. This is the document-map key a
    /// consumer verifies `EvidenceCitation.source_hash` against; it is recorded
    /// even when no citation survived, so the check is not circular.
    pub llm_input_hash: Option<String>,
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
    Ok(
        extract_structured_with_usage(markdown, Some(schema), None, llm, None)
            .await?
            .value,
    )
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
    schema: Option<&serde_json::Value>,
    user_prompt: Option<&str>,
    llm: &LlmConfig,
    max_input_bytes: Option<usize>,
) -> CrwResult<StructuredExtractResult> {
    extract_inner(markdown, schema, user_prompt, llm, max_input_bytes, None).await
}

/// Extract structured JSON **with per-field evidence** (`basis`).
///
/// Same single LLM call as [`extract_structured_with_usage`]: the model is asked
/// to attribute each top-level scalar field it extracts (source url, verbatim
/// excerpt, confidence) inside the same tool call. The attribution is then
/// verified server-side and deterministically — see [`crate::basis`]. A claim
/// that does not hold up is downgraded, never dressed up: the result never
/// carries a fake attribution.
///
/// Requires a `schema` (evidence is defined per schema leaf) and refuses
/// upfront a schema whose evidence could not fit the model's output cap.
///
/// `source_url` is the document the server fetched. It is what the citations
/// carry and what the model's claimed url is checked against; the model's own
/// string never reaches the wire.
pub async fn extract_structured_with_basis(
    markdown: &str,
    schema: &serde_json::Value,
    user_prompt: Option<&str>,
    llm: &LlmConfig,
    max_input_bytes: Option<usize>,
    source_url: &str,
) -> CrwResult<StructuredExtractResult> {
    if let Some(reason) = basis::reject_reason(schema, llm.max_tokens) {
        return Err(CrwError::InvalidRequest(reason));
    }
    extract_inner(
        markdown,
        Some(schema),
        user_prompt,
        llm,
        max_input_bytes,
        Some(source_url),
    )
    .await
}

/// The one extraction path. `basis_for` carries the document url in basis mode
/// and is `None` otherwise — and every basis behaviour (the tool-schema
/// injection, the prompt section, the longer timeout, the hash, the alignment)
/// is gated on it. With `None` the request bytes are byte-for-byte what they
/// were before basis existed, which is what keeps every existing caller and
/// self-hoster on exactly the path they are on today.
async fn extract_inner(
    markdown: &str,
    schema: Option<&serde_json::Value>,
    user_prompt: Option<&str>,
    llm: &LlmConfig,
    max_input_bytes: Option<usize>,
    basis_for: Option<&str>,
) -> CrwResult<StructuredExtractResult> {
    if llm.api_key.is_empty() {
        return Err(CrwError::ExtractionError(
            "LLM API key is empty. Set [extraction.llm.api_key] or CRW_EXTRACTION__LLM__API_KEY."
                .into(),
        ));
    }

    let max_bytes = max_input_bytes.unwrap_or(DEFAULT_MAX_INPUT_BYTES);
    let (clipped, truncated) = truncate_md(markdown, max_bytes);

    // The canonical source text is `clipped` — the exact bytes the model sees,
    // after cleaning AND truncation. Hash it BEFORE the call, so the hash can
    // never be anything the model had a hand in. (Deliberately not the same
    // hash as `ScrapeData.source_hash`, which covers the full markdown; the
    // citation's `sourceTextKind` is what disambiguates the two.)
    let llm_input_hash =
        basis_for.map(|_| format!("sha256:{}", hex::encode(Sha256::digest(clipped.as_bytes()))));

    // When the caller gave only a prompt (no schema), let the LLM shape the
    // object itself; the tool still needs an input schema, so use a permissive
    // one that accepts any properties.
    let permissive_schema = serde_json::json!({ "type": "object", "additionalProperties": true });
    let base_schema = schema.unwrap_or(&permissive_schema);
    let leaves = basis_for
        .map(|_| basis::scalar_leaves(base_schema))
        .unwrap_or_default();
    // In basis mode the caller's schema stays the document ROOT, with one
    // `basis` property added. Nesting it under a wrapper would break every
    // `"$ref": "#/$defs/..."` a generated schema carries.
    let owned_tool_schema = basis_for.map(|_| basis::tool_schema(base_schema, &leaves));
    let tool_schema = owned_tool_schema.as_ref().unwrap_or(base_schema);

    // The caller-supplied prompt (trusted API input, not scraped content) steers
    // extraction. Fall back to the generic schema-driven instruction when absent.
    let user_prompt = user_prompt.map(str::trim).filter(|p| !p.is_empty());
    let instruction = match user_prompt {
        Some(p) => format!(
            "Extract structured data from the following content. \
             Follow this instruction: {p}\n\
             Call the extract_data tool with the extracted data."
        ),
        None => "Extract structured data from the following content according to the JSON schema. \
                 Call the extract_data tool with the extracted data."
            .to_string(),
    };
    let evidence = basis_for.map(basis::prompt_section).unwrap_or_default();
    let prompt = format!("{instruction}{evidence}\n\n## Content\n{clipped}");

    let timeout = if basis_for.is_some() {
        BASIS_REQUEST_TIMEOUT
    } else {
        LLM_REQUEST_TIMEOUT
    };

    let (mut value, mut usage) = match llm.provider.as_str() {
        "anthropic" => {
            call_anthropic(
                &prompt,
                tool_schema,
                llm,
                "extract_data",
                "Extract structured data from the content",
                timeout,
            )
            .await
        }
        "openai" | "deepseek" | "openai-compatible" => {
            call_openai(
                &prompt,
                tool_schema,
                llm,
                "extract_data",
                "Extract structured data from the content",
                timeout,
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

    // Lift the basis out of the response BEFORE validation: what remains is the
    // caller's own object, which is what their schema describes. A model that
    // ignored the basis instruction entirely still produces a valid extract —
    // every leaf just lands `unsupported`. Degrade honestly, never hard-fail.
    let model_basis = basis_for
        .and_then(|_| value.as_object_mut())
        .and_then(|o| o.remove("basis"));

    // Only validate against a caller-supplied schema; a prompt-only extraction
    // has no contract to check the permissive result against.
    if let Some(schema) = schema {
        validate_against_schema(&value, schema)?;
    }

    // `value` is now schema-validated and authoritative. The model's claims are
    // checked against it and against the bytes we actually sent; they never
    // rewrite it.
    let (basis, basis_warnings) = match (basis_for, &llm_input_hash) {
        (Some(url), Some(hash)) => basis::align_basis(
            base_schema,
            &value,
            model_basis.as_ref(),
            url,
            hash,
            clipped,
        ),
        _ => (vec![], vec![]),
    };

    Ok(StructuredExtractResult {
        value,
        usage,
        truncated,
        basis,
        basis_warnings,
        llm_input_hash,
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
///
/// `timeout` is per-request (it overrides the shared client's default), because
/// a basis extraction decodes several thousand more output tokens than a judge
/// call and must not drag the judge's bound up with it.
pub(crate) async fn call_anthropic(
    prompt: &str,
    schema: &serde_json::Value,
    llm: &LlmConfig,
    tool_name: &str,
    tool_desc: &str,
    timeout: Duration,
) -> CrwResult<(serde_json::Value, Option<LlmUsage>)> {
    // D reserved lane (covers structured JSON + the change-tracking judge, which
    // both route through here). Held across the provider HTTP call.
    let _llm_permit = crate::llm_gate::acquire_llm().await;
    let url = anthropic_messages_url(llm.base_url.as_deref(), "https://api.anthropic.com");

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
        .timeout(timeout)
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
/// For a user-supplied `base_url` this matches the summary path in
/// `llm::call_openai`, so a single value works for both: the `base_url` carries
/// the API-version segment (the OpenAI `/v1` convention) and we append only
/// `/chat/completions`; a base already pointing at `…/chat/completions` is used
/// verbatim. The `None` branch intentionally diverges from the summary path:
/// here we honour the per-provider `default_base` (e.g. DeepSeek), whereas the
/// summary path hardcodes the OpenAI default — both append `/v1/chat/completions`
/// to a bare host.
///
/// This avoids the doubling bug where a `base_url` of `…/v1` became
/// `…/v1/v1/chat/completions` (→ 405) on the structured path while the summary
/// path correctly hit `…/v1/chat/completions`.
fn openai_chat_url(base_url: Option<&str>, default_base: &str) -> String {
    match base_url {
        Some(b) if b.contains("/chat/completions") => b.to_string(),
        Some(b) => format!("{}/chat/completions", b.trim_end_matches('/')),
        None => format!("{}/v1/chat/completions", default_base.trim_end_matches('/')),
    }
}

/// Resolve the messages endpoint for an Anthropic-compatible provider.
///
/// The same bug class as `openai_chat_url`: the Anthropic **summary** path
/// (`llm::call_anthropic`) consumes `base_url` verbatim (its default is the full
/// `…/v1/messages`), so a bare host that works here would 404 there. To let a
/// single full `…/v1/messages` endpoint satisfy both paths, this is idempotent:
/// a base already ending in `/v1/messages` is used verbatim; one ending in `/v1`
/// gets `/messages`; a bare host (or `None`) gets the full `/v1/messages` suffix.
/// We treat a base as already-complete only when `/v1/messages` is its true
/// suffix (`ends_with`, after trimming a trailing slash) rather than merely
/// present (`contains`), so a path that happens to embed `/v1/messages`
/// mid-string is not mistaken for a finished endpoint.
fn anthropic_messages_url(base_url: Option<&str>, default_base: &str) -> String {
    let b = match base_url {
        Some(b) => b,
        None => return format!("{}/v1/messages", default_base.trim_end_matches('/')),
    };
    let trimmed = b.trim_end_matches('/');
    if trimmed.ends_with("/v1/messages") {
        trimmed.to_string()
    } else if trimmed.ends_with("/v1") {
        format!("{trimmed}/messages")
    } else {
        format!("{trimmed}/v1/messages")
    }
}

/// Call an OpenAI-compatible provider with a function-call forcing the given
/// `schema`. `prompt` is the full user message; `tool_name`/`tool_desc` name
/// the forced function. Shared by structured extraction and the judge.
///
/// `timeout` is per-request — see [`call_anthropic`].
pub(crate) async fn call_openai(
    prompt: &str,
    schema: &serde_json::Value,
    llm: &LlmConfig,
    tool_name: &str,
    tool_desc: &str,
    timeout: Duration,
) -> CrwResult<(serde_json::Value, Option<LlmUsage>)> {
    // D reserved lane (structured JSON + judge). Held across the HTTP call.
    let _llm_permit = crate::llm_gate::acquire_llm().await;
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
        .timeout(timeout)
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
    fn openai_url_base_ending_in_v1_is_not_doubled() {
        // Regression for the structured-extraction doubling bug: a base_url
        // ending in `/v1` (the OpenAI convention, exactly as the summary path
        // in `llm::call_openai` treats it) must append only `/chat/completions`
        // — never a second `/v1`. Otherwise structured extraction hits
        // `…/v1/v1/chat/completions` (→ 405) while summary hits the right URL.
        assert_eq!(
            openai_chat_url(Some("http://gateway:8080/v1"), "https://api.openai.com"),
            "http://gateway:8080/v1/chat/completions"
        );
    }

    #[test]
    fn openai_url_appends_path_to_base() {
        // The base_url carries the API-version segment (`/v1`), matching the
        // summary path: we only append `/chat/completions`.
        assert_eq!(
            openai_chat_url(
                Some("https://api.deepseek.com/v1"),
                "https://api.openai.com"
            ),
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
        // The default base is a bare host, so it still gets the full
        // `/v1/chat/completions` suffix.
        assert_eq!(
            openai_chat_url(None, "https://api.openai.com"),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn openai_url_trims_trailing_slash() {
        assert_eq!(
            openai_chat_url(
                Some("https://api.deepseek.com/v1/"),
                "https://api.openai.com"
            ),
            "https://api.deepseek.com/v1/chat/completions"
        );
    }

    #[test]
    fn anthropic_url_bare_host_gets_full_suffix() {
        // No user base_url: the bare default gets the full `/v1/messages` suffix.
        assert_eq!(
            anthropic_messages_url(None, "https://api.anthropic.com"),
            "https://api.anthropic.com/v1/messages"
        );
        // A bare-host base_url behaves the same.
        assert_eq!(
            anthropic_messages_url(Some("https://proxy.internal"), "https://api.anthropic.com"),
            "https://proxy.internal/v1/messages"
        );
    }

    #[test]
    fn anthropic_url_base_ending_in_v1_is_not_doubled() {
        // Same bug class as the OpenAI path: a base ending in `/v1` must get only
        // `/messages` appended — never a second `/v1`.
        assert_eq!(
            anthropic_messages_url(
                Some("https://proxy.internal/v1"),
                "https://api.anthropic.com"
            ),
            "https://proxy.internal/v1/messages"
        );
        assert_eq!(
            anthropic_messages_url(
                Some("https://proxy.internal/v1/"),
                "https://api.anthropic.com"
            ),
            "https://proxy.internal/v1/messages"
        );
    }

    #[test]
    fn anthropic_url_full_endpoint_is_verbatim() {
        // A full `…/v1/messages` endpoint — the value that also satisfies the
        // summary path (which uses base_url verbatim) — is used as-is, not doubled.
        assert_eq!(
            anthropic_messages_url(
                Some("https://proxy.internal/v1/messages"),
                "https://api.anthropic.com"
            ),
            "https://proxy.internal/v1/messages"
        );
        assert_eq!(
            anthropic_messages_url(
                Some("https://proxy.internal/v1/messages/"),
                "https://api.anthropic.com"
            ),
            "https://proxy.internal/v1/messages"
        );
    }
}
