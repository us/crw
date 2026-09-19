//! `POST /v2/search` — reuses the v1 `search_inner` engine, reshaping the
//! response into the v2 envelope `{ success, data: {web,news,images}, creditsUsed, id }`.

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crw_core::error::CrwError;
use crw_core::types::{ImageResult, LlmUsage, SearchData, SearchRequest, SearchResult};

use super::error::V2Error;
use crate::error::AppError;
use crate::routes::search::search_inner;
use crate::state::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct V2SearchResponse {
    pub success: bool,
    pub data: V2SearchData,
    pub credits_used: u32,
    pub id: String,
    /// Token usage for the LLM legs this search ran (summaries, answer).
    ///
    /// Additive to the frozen Firecrawl envelope, which their SDKs ignore, and
    /// the same argument as `V2Document::llm_usage`: a caller metering managed
    /// spend off this surface had no way to see what a search cost, because
    /// the reshape below kept only `results`. `/v1/search` has always carried
    /// it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_usage: Option<LlmUsage>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct V2SearchData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web: Option<Vec<SearchResult>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news: Option<Vec<SearchResult>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<ImageResult>>,
}

/// v2 `scrapeOptions.formats` may be objects; the v1 `SearchRequest` only
/// accepts string formats. Rewrite the formats array to strings (lifting a
/// `json` schema to `jsonSchema`) before deserializing into `SearchRequest`.
fn normalize_search_body(mut v: Value) -> Value {
    if let Some(opts) = v.get_mut("scrapeOptions").and_then(Value::as_object_mut)
        && let Some(Value::Array(arr)) = opts.get("formats").cloned()
    {
        let mut strs = Vec::new();
        let mut schema: Option<Value> = None;
        for f in arr {
            match f {
                Value::String(s) => strs.push(Value::String(s)),
                Value::Object(m) => {
                    if let Some(t) = m.get("type").and_then(Value::as_str) {
                        strs.push(Value::String(t.to_string()));
                        if t == "json"
                            && let Some(s) = m.get("schema")
                        {
                            schema = Some(s.clone());
                        }
                    }
                }
                _ => {}
            }
        }
        opts.insert("formats".to_string(), Value::Array(strs));
        if let Some(s) = schema {
            opts.entry("jsonSchema".to_string()).or_insert(s);
        }
    }
    v
}

fn shape(results: SearchData) -> V2SearchData {
    match results {
        SearchData::Flat(v) => V2SearchData {
            web: Some(v),
            ..Default::default()
        },
        SearchData::Grouped(g) => V2SearchData {
            web: g.web,
            news: g.news,
            images: g.images,
        },
    }
}

pub async fn search(
    State(state): State<AppState>,
    body: Result<Json<Value>, JsonRejection>,
) -> Result<Json<V2SearchResponse>, V2Error> {
    let Json(raw) = body.map_err(AppError::from)?;
    let normalized = normalize_search_body(raw);
    let req: SearchRequest = serde_json::from_value(normalized)
        .map_err(|e| CrwError::InvalidRequest(format!("Invalid search request: {e}")))?;

    let resp = search_inner(&state, req).await?;
    // Destructure rather than reshaping in place: `SearchResponseData` carries
    // `llm_usage` next to `results`, and mapping straight to `shape(d.results)`
    // dropped it on the floor.
    let (data, llm_usage) = match resp.data {
        Some(d) => (shape(d.results), d.llm_usage),
        None => (V2SearchData::default(), None),
    };

    Ok(Json(V2SearchResponse {
        success: true,
        data,
        llm_usage,
        credits_used: 0,
        id: Uuid::new_v4().to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: u32, output: u32) -> LlmUsage {
        LlmUsage {
            input_tokens: input,
            output_tokens: output,
            total_tokens: input + output,
            estimated_cost_usd: None,
            model: "DeepSeek-V4-Pro".to_string(),
            provider: "openai-compatible".to_string(),
            cache_hit_input_tokens: None,
            cache_miss_input_tokens: None,
            truncated: false,
            calls: 1,
            executed_summaries: 2,
            answer_executed: true,
        }
    }

    fn response(llm_usage: Option<LlmUsage>) -> V2SearchResponse {
        V2SearchResponse {
            success: true,
            data: V2SearchData::default(),
            llm_usage,
            credits_used: 0,
            id: "id".to_string(),
        }
    }

    #[test]
    fn v2_search_carries_llm_usage_on_the_wire() {
        // `/v2/search` reshapes the inner response down to its results, which
        // dropped the usage the summarize and answer legs reported. A caller
        // metering managed spend on this surface could not see what a search
        // cost. The key must be camelCase, because that is what reads it.
        let wire = serde_json::to_value(response(Some(usage(1200, 340)))).unwrap();
        let u = wire
            .get("llmUsage")
            .expect("llmUsage must reach the wire when a model ran");
        assert_eq!(u.get("inputTokens").unwrap(), 1200);
        assert_eq!(u.get("outputTokens").unwrap(), 340);
        assert_eq!(u.get("executedSummaries").unwrap(), 2);
        assert_eq!(u.get("answerExecuted").unwrap(), true);
    }

    #[test]
    fn v2_search_omits_llm_usage_when_no_model_ran() {
        // A plain search must keep the Firecrawl envelope byte-identical: no
        // `"llmUsage": null` key appearing where their SDKs never saw one.
        let wire = serde_json::to_value(response(None)).unwrap();
        assert!(
            wire.get("llmUsage").is_none(),
            "a search with no LLM leg must not grow the key"
        );
    }
}
