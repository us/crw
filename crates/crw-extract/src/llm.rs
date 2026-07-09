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
use rand::RngExt;
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
        None, // extraction path: keep provider-default temperature
        None, // extraction path: no reasoning_effort
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
        cfg.temperature,
        cfg.reasoning_effort.as_deref(),
        system_prompt,
        user_msg,
    )
    .await
}

/// Generate ONE entity/keyword-focused rewrite of a search query to widen
/// retrieval recall on the answer path. The caller fetches BOTH the original
/// and this rewrite and unions the candidate pools, so recall can only
/// increase. Returns an empty `Vec` on any failure or when the rewrite is
/// trivial/identical — the caller then uses the original query alone, which
/// means this can never reduce recall or break a search.
pub async fn expand_query(cfg: &LlmConfig, query: &str, max_variants: usize) -> Vec<String> {
    let n = max_variants.max(1);
    let sys = format!(
        "You rewrite a user's search query into up to {n} alternative \
         web-search queries that maximize the chance of finding the answer. \
         Rules: (1) EXPAND any abbreviation, acronym, or initialism to its full \
         proper name. (2) Keep the key named entities; use precise keywords a \
         relevant page would contain; drop filler words — but ALWAYS keep any \
         place name, city, region, or country VERBATIM in EVERY rewrite. A \
         location is never a filler word: dropping \"belgrade\" from \"best \
         pizza in belgrade\" would surface the wrong city, so preserve it in \
         all variants. (3) Make the alternatives DIVERSE — e.g. one focused on \
         the full entity name, one on distinctive keywords. Output ONLY the \
         rewritten queries, ONE per line: no quotes, no numbering, no labels. \
         Output at most {n} line(s)."
    );
    let mut leg = cfg.clone();
    leg.max_tokens = leg.max_tokens.min(60 + 60 * n as u32);
    match chat(&leg, &sys, query).await {
        Ok(r) => {
            let mut out: Vec<String> = Vec::new();
            for line in r.content.trim().lines() {
                let v = line.trim().trim_matches('"').trim().to_string();
                if v.is_empty() || v.eq_ignore_ascii_case(query.trim()) {
                    continue;
                }
                if out.iter().any(|e| e.eq_ignore_ascii_case(&v)) {
                    continue;
                }
                out.push(v);
                if out.len() >= n {
                    break;
                }
            }
            out
        }
        Err(_) => Vec::new(),
    }
}

/// Evidence-scout for adaptive multi-round retrieval. Given the question and a
/// short excerpt of what round-1 retrieval surfaced (which did NOT answer it),
/// produce up to `max_queries` TARGETED follow-up web-search queries to find or
/// confirm the answer. Unlike `expand_query` (blind rephrasings), the scout is
/// failure-aware: it leans on entity names/aliases seen in the evidence and
/// goes harder — exact-phrase `"entity"`, full official names for any acronym,
/// the specific predicate/date asked, or a likely authoritative source. Returns
/// deduped queries (empty on LLM failure → caller simply skips the extra round).
pub async fn scout_followups(
    cfg: &LlmConfig,
    query: &str,
    evidence: &str,
    max_queries: usize,
) -> Vec<String> {
    let n = max_queries.max(1);
    let sys = format!(
        "A first web search did NOT answer the user's question. You are a search \
         strategist. Using the question and the EVIDENCE excerpt of what the first \
         search found, write up to {n} NEW, BETTER web-search queries likely to \
         surface or confirm the answer. Rules: (1) EXPAND every acronym/initialism \
         to its full proper name. (2) Prefer the exact entity name(s) seen in the \
         evidence, quoted, plus the specific thing asked (the predicate, the date, \
         the number). ALWAYS keep any place name, city, region, or country from \
         the question VERBATIM in every query — a location is never optional. \
         (3) Try a different angle than the original phrasing — an exact-phrase \
         query, an authoritative source guess, or the canonical entity. (4) Do NOT \
         repeat the user's original wording. Output ONLY the queries, ONE per \
         line: no quotes around the whole line, no numbering, no labels. Output at \
         most {n} line(s)."
    );
    let user = format!("QUESTION: {query}\n\nEVIDENCE (did not answer it):\n{evidence}");
    let mut leg = cfg.clone();
    leg.max_tokens = leg.max_tokens.min(60 + 60 * n as u32);
    match chat(&leg, &sys, &user).await {
        Ok(r) => {
            let mut out: Vec<String> = Vec::new();
            for line in r.content.trim().lines() {
                let v = line.trim().trim_matches('"').trim().to_string();
                if v.is_empty() || v.eq_ignore_ascii_case(query.trim()) {
                    continue;
                }
                if out.iter().any(|e| e.eq_ignore_ascii_case(&v)) {
                    continue;
                }
                out.push(v);
                if out.len() >= n {
                    break;
                }
            }
            out
        }
        Err(_) => Vec::new(),
    }
}

#[allow(clippy::too_many_arguments)]
async fn dispatch(
    provider: &str,
    api_key: &str,
    model: &str,
    base_url: Option<&str>,
    max_tokens: u32,
    azure_api_version: Option<&str>,
    temperature: Option<f32>,
    reasoning_effort: Option<&str>,
    system_prompt: &str,
    user_msg: &str,
) -> CrwResult<LlmCallResult> {
    // D reserved lane: bound LLM-call concurrency and keep a slice for
    // interactive traffic. Read the class here (async side) and hold the permit
    // across the provider HTTP call.
    let _llm_permit = crate::llm_gate::acquire_llm().await;
    let client = shared_client();
    match provider.to_ascii_lowercase().as_str() {
        "anthropic" => {
            call_anthropic(
                client,
                api_key,
                model,
                base_url,
                max_tokens,
                temperature,
                system_prompt,
                user_msg,
            )
            .await
        }
        // DeepSeek and other OpenAI-compatible providers use the same wire
        // protocol; users select them via `base_url`. Thread the dispatcher's
        // provider tag through so usage records reflect the actual provider
        // (e.g. `deepseek`) instead of always reporting `openai`.
        provider_tag @ ("openai" | "deepseek" | "openai-compatible") => {
            call_openai(
                client,
                api_key,
                model,
                base_url,
                max_tokens,
                temperature,
                reasoning_effort,
                system_prompt,
                user_msg,
                provider_tag,
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
                temperature,
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
    temperature: Option<f32>,
    system_prompt: &str,
    user_msg: &str,
) -> CrwResult<LlmCallResult> {
    let url = base_url.unwrap_or(ANTHROPIC_DEFAULT_BASE_URL);
    let mut body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "system": system_prompt,
        "messages": [{ "role": "user", "content": user_msg }],
    });
    if let Some(t) = temperature {
        body["temperature"] = serde_json::json!(t);
    }
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
    temperature: Option<f32>,
    reasoning_effort: Option<&str>,
    system_prompt: &str,
    user_msg: &str,
    provider: &str,
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
    let mut body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": user_msg }
        ],
    });
    // Deterministic eval: temp=0 + fixed seed make answers reproducible so a
    // real +2-3pp lever is distinguishable from sampling noise. None (prod
    // default) sends neither, preserving the provider default.
    if let Some(t) = temperature {
        body["temperature"] = serde_json::json!(t);
        body["seed"] = serde_json::json!(42);
    }
    // Only forward a present, non-empty value. A configured-but-empty value
    // deserializes to `Some("")` and would be rejected (HTTP 400) by providers
    // that validate the field, so treat it as unset.
    if let Some(effort) = reasoning_effort.filter(|s| !s.is_empty()) {
        body["reasoning_effort"] = serde_json::json!(effort);
    }
    // Fixed self-limiting retry on transient server throttling (HTTP 429/503)
    // only. The shared client carries a 30s per-attempt timeout and the
    // caller's request deadline is not threaded in here, so the budget stays
    // small and fixed (a few short jittered sleeps). 429/503 are fast server
    // rejects, so the worst case stays well under the request deadline. All
    // other non-2xx responses (and transport/timeout errors) keep the original
    // single-POST contract: hard-error on the first response.
    const MAX_ATTEMPTS: u32 = 3;
    let mut attempt: u32 = 0;
    let (status, text) = loop {
        attempt += 1;
        let resp = client
            .post(url)
            .bearer_auth(api_key)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| CrwError::Internal(format!("LLM request failed: {e}")))?;

        let status = resp.status();
        let is_retryable = status == reqwest::StatusCode::TOO_MANY_REQUESTS
            || status == reqwest::StatusCode::SERVICE_UNAVAILABLE;
        if is_retryable && attempt < MAX_ATTEMPTS {
            // Exponential backoff with jitter: ~0.5s, ~1s base + up to ~1s
            // jitter. Drop the response body unread — the status is enough.
            let base_ms = 500u64 * (1u64 << (attempt - 1));
            let jitter_ms = rand::rng().random_range(0..1000);
            tokio::time::sleep(Duration::from_millis(base_ms + jitter_ms)).await;
            continue;
        }

        let text = resp
            .text()
            .await
            .map_err(|e| CrwError::Internal(format!("LLM response read failed: {e}")))?;
        break (status, text);
    };
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

    let usage = parse_openai_usage(&payload, model, provider);
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
    temperature: Option<f32>,
    system_prompt: &str,
    user_msg: &str,
) -> CrwResult<LlmCallResult> {
    let endpoint_trimmed = endpoint.trim_end_matches('/');
    let url = format!(
        "{endpoint_trimmed}/openai/deployments/{deployment}/chat/completions?api-version={api_version}"
    );
    let mut body = serde_json::json!({
        "max_tokens": max_tokens,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": user_msg }
        ],
    });
    if let Some(t) = temperature {
        body["temperature"] = serde_json::json!(t);
        body["seed"] = serde_json::json!(42);
    }
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
    // Anthropic prompt-cache fields (only present when cache is in use).
    // `cache_read_input_tokens` is a cache HIT (discounted read).
    // `cache_creation_input_tokens` is a cache WRITE — billed at the full
    // input rate, so we count it as a "miss" for the hit/miss breakdown.
    let cache_read = usage
        .get("cache_read_input_tokens")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);
    let cache_creation = usage
        .get("cache_creation_input_tokens")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);
    let (cache_hit_input_tokens, cache_miss_input_tokens) = match (cache_read, cache_creation) {
        (None, None) => (None, None),
        (read, create) => {
            let hit = read.unwrap_or(0);
            let create = create.unwrap_or(0);
            // Anthropic reports `input_tokens` as non-cached input only —
            // the cache_read/cache_creation buckets are additive on top.
            // Treat plain `input_tokens` + cache_creation as the miss side.
            let miss = input_tokens.saturating_add(create);
            (Some(hit), Some(miss))
        }
    };
    let total = input_tokens + output_tokens;
    Some(LlmUsage {
        input_tokens,
        output_tokens,
        total_tokens: total,
        estimated_cost_usd: pricing::calculate_cost(model, input_tokens, output_tokens),
        model: model.to_string(),
        provider: "anthropic".to_string(),
        cache_hit_input_tokens,
        cache_miss_input_tokens,
        truncated: false,
        calls: 1,
        // R1 counters are scoped to /v1/search aggregation; single-call
        // sites always emit defaults. Aggregation happens in the caller
        // (crw-server::routes::search::search_inner).
        executed_summaries: 0,
        answer_executed: false,
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

    // Cache breakdown — providers expose this two different ways:
    //   * OpenAI / Azure / OpenAI-compat: `usage.prompt_tokens_details.cached_tokens`
    //     (cache_hit; cache_miss = prompt_tokens - cached_tokens)
    //   * DeepSeek: explicit `prompt_cache_hit_tokens` / `prompt_cache_miss_tokens`
    //     at the top level of `usage`.
    let deepseek_hit = usage
        .get("prompt_cache_hit_tokens")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);
    let deepseek_miss = usage
        .get("prompt_cache_miss_tokens")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);
    let openai_cached = usage
        .get("prompt_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);

    let (cache_hit_input_tokens, cache_miss_input_tokens) =
        match (deepseek_hit, deepseek_miss, openai_cached) {
            (Some(hit), Some(miss), _) => (Some(hit), Some(miss)),
            (Some(hit), None, _) => (Some(hit), Some(input_tokens.saturating_sub(hit))),
            (None, Some(miss), _) => (Some(input_tokens.saturating_sub(miss)), Some(miss)),
            (None, None, Some(cached)) => (Some(cached), Some(input_tokens.saturating_sub(cached))),
            (None, None, None) => (None, None),
        };

    Some(LlmUsage {
        input_tokens,
        output_tokens,
        total_tokens: total,
        estimated_cost_usd: pricing::calculate_cost(model, input_tokens, output_tokens),
        model: model.to_string(),
        provider: provider.to_string(),
        cache_hit_input_tokens,
        cache_miss_input_tokens,
        truncated: false,
        calls: 1,
        // R1 counters are scoped to /v1/search aggregation; single-call
        // sites always emit defaults. Aggregation happens in the caller
        // (crw-server::routes::search::search_inner).
        executed_summaries: 0,
        answer_executed: false,
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
        assert_eq!(usage.calls, 1);
        assert!(!usage.truncated);
        assert!(usage.cache_hit_input_tokens.is_none());
        assert!(usage.cache_miss_input_tokens.is_none());
    }

    #[test]
    fn parse_anthropic_usage_extracts_cache_hit_tokens() {
        let payload = serde_json::json!({
            "usage": {
                "input_tokens": 80,
                "output_tokens": 40,
                "cache_read_input_tokens": 1024,
                "cache_creation_input_tokens": 256,
            }
        });
        let usage = parse_anthropic_usage(&payload, "claude-haiku-4-5").unwrap();
        assert_eq!(usage.input_tokens, 80);
        assert_eq!(usage.output_tokens, 40);
        assert_eq!(usage.cache_hit_input_tokens, Some(1024));
        // miss = plain input_tokens + cache_creation (both billed at full rate)
        assert_eq!(usage.cache_miss_input_tokens, Some(80 + 256));
        assert_eq!(usage.provider, "anthropic");
        assert_eq!(usage.calls, 1);
    }

    #[test]
    fn parse_openai_usage_deepseek_cache_breakdown() {
        // DeepSeek-style explicit hit/miss fields.
        let payload = serde_json::json!({
            "usage": {
                "prompt_tokens": 1500,
                "completion_tokens": 200,
                "total_tokens": 1700,
                "prompt_cache_hit_tokens": 1200,
                "prompt_cache_miss_tokens": 300,
            }
        });
        let usage = parse_openai_usage(&payload, "deepseek-chat", "deepseek").unwrap();
        assert_eq!(usage.input_tokens, 1500);
        assert_eq!(usage.cache_hit_input_tokens, Some(1200));
        assert_eq!(usage.cache_miss_input_tokens, Some(300));
        // Provider tag must be carried through — NOT hardcoded to "openai".
        assert_eq!(usage.provider, "deepseek");
    }

    #[test]
    fn parse_openai_usage_compat_cached_tokens() {
        // OpenAI / OpenAI-compatible style: nested prompt_tokens_details.cached_tokens.
        let payload = serde_json::json!({
            "usage": {
                "prompt_tokens": 1000,
                "completion_tokens": 50,
                "total_tokens": 1050,
                "prompt_tokens_details": { "cached_tokens": 400 },
            }
        });
        let usage = parse_openai_usage(&payload, "gpt-4o-mini", "openai").unwrap();
        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.cache_hit_input_tokens, Some(400));
        assert_eq!(usage.cache_miss_input_tokens, Some(600));
        assert_eq!(usage.provider, "openai");
    }
}
