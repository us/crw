use crw_core::config::LlmConfig;
use crw_extract::structured::extract_structured;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The structured path runs with the SERVER's LLM key on the managed
/// configuration, so a gateway that mirrors the request into its error body
/// would hand that key (and the prompt) to whoever called the public API.
/// The HTTP status is all the caller gets.
#[tokio::test]
async fn provider_error_body_is_never_echoed_to_the_caller() {
    for (provider, api_path) in [
        ("openai", "/v1/chat/completions"),
        ("anthropic", "/v1/messages"),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(api_path))
            .respond_with(ResponseTemplate::new(400).set_body_string(
                "{\"error\":\"echoed Authorization: Bearer SENTINEL-LEAK-TOKEN\"}",
            ))
            .mount(&server)
            .await;

        let llm = LlmConfig {
            provider: provider.into(),
            api_key: "test-key".into(),
            model: "test-model".into(),
            base_url: Some(format!("{}/v1", server.uri())),
            max_tokens: 256,
            ..Default::default()
        };
        let schema = json!({
            "type": "object",
            "properties": { "name": { "type": "string" } },
            "required": ["name"]
        });

        let err = extract_structured("content", &schema, &llm)
            .await
            .expect_err("a 400 from the provider must error");
        let msg = err.to_string();
        assert!(msg.contains("400"), "{provider}: status is surfaced: {msg}");
        assert!(
            !msg.contains("SENTINEL-LEAK-TOKEN"),
            "{provider}: provider error body must not be echoed: {msg}"
        );
    }
}

#[test]
fn parse_json_response_invalid_json() {
    // parse_json_response is private, so we test schema validation behavior.
    let schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" }
        },
        "required": ["name"]
    });
    // Missing required field "name" should fail validation
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(&json!({}))
        .map(|e| e.to_string())
        .collect();
    assert!(!errors.is_empty(), "Missing required field should fail");
}

#[test]
fn validate_schema_empty_object() {
    // Empty object is valid if no required fields
    let schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" }
        }
    });
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(&json!({}))
        .map(|e| e.to_string())
        .collect();
    assert!(
        errors.is_empty(),
        "Empty object should be valid: {errors:?}"
    );
}
