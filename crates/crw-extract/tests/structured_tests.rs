use serde_json::json;

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
