//! Wire-level coverage for the OpenAI Responses provider. These tests lock the
//! protocol boundary separately from the existing Chat Completions tests.

use crw_core::config::LlmConfig;
use crw_extract::llm::chat;
use crw_extract::structured::extract_structured;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn responses_llm(base_url: String) -> LlmConfig {
    LlmConfig {
        provider: "openai-responses".into(),
        api_key: "test-key".into(),
        model: "test-model".into(),
        base_url: Some(base_url),
        max_tokens: 512,
        ..Default::default()
    }
}

#[tokio::test]
async fn text_call_uses_responses_wire_shape_and_parses_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "resp_test",
            "status": "completed",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": "hello from provider" }]
            }],
            "usage": {
                "input_tokens": 20,
                "output_tokens": 5,
                "total_tokens": 25,
                "input_tokens_details": { "cached_tokens": 8 }
            }
        })))
        .mount(&server)
        .await;

    let mut llm = responses_llm(format!("{}/v1", server.uri()));
    llm.reasoning_effort = Some("low".into());
    let result = chat(&llm, "trusted instructions", "user input")
        .await
        .expect("Responses text call succeeds");

    assert_eq!(result.content, "hello from provider");
    let usage = result.usage.expect("usage is surfaced");
    assert_eq!(usage.provider, "openai-responses");
    assert_eq!(usage.input_tokens, 20);
    assert_eq!(usage.cache_hit_input_tokens, Some(8));
    assert_eq!(usage.cache_miss_input_tokens, Some(12));

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], "test-model");
    assert_eq!(body["instructions"], "trusted instructions");
    assert_eq!(body["input"], "user input");
    assert_eq!(body["max_output_tokens"], 512);
    assert_eq!(body["store"], false);
    assert_eq!(body["reasoning"]["effort"], "low");
    assert!(body.get("messages").is_none());
}

#[tokio::test]
async fn structured_call_forces_flat_function_tool_and_parses_arguments() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "resp_structured",
            "status": "completed",
            "output": [{
                "type": "function_call",
                "id": "fc_1",
                "call_id": "call_1",
                "name": "extract_data",
                "arguments": "{\"name\":\"Ada\",\"count\":3}",
                "status": "completed"
            }],
            "usage": { "input_tokens": 100, "output_tokens": 12, "total_tokens": 112 }
        })))
        .mount(&server)
        .await;

    let llm = responses_llm(format!("{}/v1/", server.uri()));
    let schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "count": { "type": "integer" }
        },
        "required": ["name", "count"]
    });
    let value = extract_structured("Ada has 3 entries.", &schema, &llm)
        .await
        .expect("Responses structured extraction succeeds");
    assert_eq!(value, json!({ "name": "Ada", "count": 3 }));

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["tools"][0]["type"], "function");
    assert_eq!(body["tools"][0]["name"], "extract_data");
    assert_eq!(body["tools"][0]["parameters"], schema);
    assert_eq!(
        body["tool_choice"],
        json!({ "type": "function", "name": "extract_data" })
    );
    assert_eq!(body["parallel_tool_calls"], false);
    assert!(body["tools"][0].get("function").is_none());
}

#[tokio::test]
async fn structured_call_still_validates_function_arguments_locally() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "output": [{
                "type": "function_call",
                "name": "extract_data",
                "arguments": "{\"count\":\"not-an-integer\"}"
            }]
        })))
        .mount(&server)
        .await;

    let llm = responses_llm(format!("{}/v1", server.uri()));
    let schema = json!({
        "type": "object",
        "properties": { "count": { "type": "integer" } },
        "required": ["count"]
    });
    let error = extract_structured("count", &schema, &llm)
        .await
        .expect_err("schema-invalid arguments must fail");
    assert!(error.to_string().contains("schema validation"));
}
