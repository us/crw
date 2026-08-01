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

/// Mount one Responses payload and run a text call against it.
async fn text_call_returning(
    payload: serde_json::Value,
) -> (
    MockServer,
    Result<crw_extract::llm::LlmCallResult, crw_core::error::CrwError>,
) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(payload))
        .mount(&server)
        .await;

    let llm = responses_llm(format!("{}/v1", server.uri()));
    let result = chat(&llm, "sys", "user").await;
    (server, result)
}

fn message_output(text: &str) -> serde_json::Value {
    json!([{
        "type": "message",
        "role": "assistant",
        "content": [{ "type": "output_text", "text": text }]
    }])
}

#[tokio::test]
async fn failed_status_inside_http_200_is_an_error_not_a_result() {
    // The Responses API reports terminal failure in the payload, so an HTTP 200
    // is not by itself a success. The gateway-written error body must not ride
    // along into the caller-facing error — same rule as the non-2xx path.
    let (_server, result) = text_call_returning(json!({
        "status": "failed",
        "error": {
            "code": "server_error",
            "message": "rejected: Authorization: Bearer SENTINEL-LEAK-TOKEN"
        },
        "output": []
    }))
    .await;

    let err = result.expect_err("failed status must not be reported as success");
    let msg = err.to_string();
    assert!(msg.contains("failed"), "status is surfaced: {msg}");
    assert!(
        !msg.contains("SENTINEL-LEAK-TOKEN"),
        "provider error body must not be echoed: {msg}"
    );
}

#[tokio::test]
async fn failed_status_without_a_message_still_errors() {
    let (_server, result) = text_call_returning(json!({ "status": "failed", "output": [] })).await;
    let err = result.expect_err("failed status must error even with no error.message");
    assert!(err.to_string().contains("failed"));
}

#[tokio::test]
async fn non_terminal_status_is_not_mistaken_for_output() {
    // A gateway emitting a still-running response must not read as an empty
    // success — the call produced nothing.
    for status in ["queued", "in_progress", "cancelled"] {
        let (_server, result) = text_call_returning(json!({
            "status": status,
            "output": []
        }))
        .await;
        let err = result.expect_err("non-terminal status must error");
        assert!(err.to_string().contains(status), "status named in error");
    }
}

#[tokio::test]
async fn incomplete_status_keeps_the_partial_output() {
    // Stopped at max_output_tokens: same contract as Chat Completions'
    // finish_reason == "length", which is not an error either.
    let (_server, result) = text_call_returning(json!({
        "status": "incomplete",
        "incomplete_details": { "reason": "max_output_tokens" },
        "output": message_output("partial answer"),
        "usage": { "input_tokens": 10, "output_tokens": 4, "total_tokens": 14 }
    }))
    .await;

    let ok = result.expect("incomplete response still carries usable content");
    assert_eq!(ok.content, "partial answer");
    // `truncated` means "the markdown INPUT was clipped before the call" for
    // every provider. An output-side cut must not repurpose it, or the field
    // would mean two different things depending on the provider.
    let usage = ok.usage.expect("usage is surfaced");
    assert!(!usage.truncated, "output-side cut must not set `truncated`");
}

#[tokio::test]
async fn message_with_empty_text_succeeds_like_the_chat_completions_path() {
    // Sibling providers treat a present-but-empty content string as a valid
    // response; only a missing one errors. Switching provider must not turn a
    // working request into a hard failure.
    let (_server, result) = text_call_returning(json!({
        "status": "completed",
        "output": message_output("")
    }))
    .await;

    let ok = result.expect("empty-but-present content is a success");
    assert_eq!(ok.content, "");
}

#[tokio::test]
async fn response_without_any_message_item_errors() {
    // Nothing textual was produced (reasoning-only): that is "content absent",
    // which the sibling paths reject too.
    let (_server, result) = text_call_returning(json!({
        "status": "completed",
        "output": [{ "type": "reasoning", "summary": [] }]
    }))
    .await;

    let err = result.expect_err("a response with no message item must error");
    assert!(err.to_string().contains("missing output_text"));
}

#[tokio::test]
async fn refusal_only_response_errors_instead_of_returning_an_empty_answer() {
    // A refusal arrives as a message whose content holds no output_text part.
    // Chat Completions nulls `content` for the same case and errors, so this
    // must not surface as a successful empty answer.
    let (_server, result) = text_call_returning(json!({
        "status": "completed",
        "output": [{
            "type": "message",
            "role": "assistant",
            "content": [{ "type": "refusal", "refusal": "I cannot help with that." }]
        }]
    }))
    .await;

    let err = result.expect_err("a refusal-only response must error");
    assert!(err.to_string().contains("missing output_text"));
}

#[tokio::test]
async fn http_error_body_is_never_echoed_to_the_caller() {
    // A gateway that mirrors the request back in its error body would otherwise
    // hand the bearer key or the prompt to the API caller.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(400).set_body_string(
            "{\"error\":{\"message\":\"echoed Authorization: Bearer SENTINEL-LEAK-TOKEN\"}}",
        ))
        .mount(&server)
        .await;

    let llm = responses_llm(format!("{}/v1", server.uri()));
    let err = chat(&llm, "sys", "user")
        .await
        .expect_err("non-2xx must error");

    let msg = err.to_string();
    assert!(msg.contains("400"), "status is surfaced: {msg}");
    assert!(
        !msg.contains("SENTINEL-LEAK-TOKEN"),
        "error body must not be echoed: {msg}"
    );
}
