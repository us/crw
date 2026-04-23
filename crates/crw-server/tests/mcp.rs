use axum_test::TestServer;
use crw_core::config::AppConfig;
use crw_server::app::create_app;
use crw_server::state::AppState;
use serde_json::json;

fn test_app() -> TestServer {
    let config: AppConfig = toml::from_str("").unwrap();
    let state = AppState::new(config).expect("AppState::new failed");
    let app = create_app(state);
    TestServer::new(app)
}

fn mcp_request(
    method: &str,
    id: serde_json::Value,
    params: serde_json::Value,
) -> serde_json::Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    })
}

#[tokio::test]
async fn mcp_initialize_returns_capabilities() {
    let server = test_app();
    let resp = server
        .post("/mcp")
        .content_type("application/json")
        .json(&mcp_request("initialize", json!(1), json!({})))
        .await;
    resp.assert_status_ok();
    let json: serde_json::Value = resp.json();
    assert_eq!(json["jsonrpc"], "2.0");
    assert_eq!(json["id"], 1);
    let result = &json["result"];
    assert!(result["protocolVersion"].is_string());
    assert!(result["capabilities"].is_object());
    assert!(result["serverInfo"]["name"].is_string());
    assert!(result["serverInfo"]["version"].is_string());
}

#[tokio::test]
async fn mcp_tools_list_returns_4_tools() {
    let server = test_app();
    let resp = server
        .post("/mcp")
        .content_type("application/json")
        .json(&mcp_request("tools/list", json!(2), json!({})))
        .await;
    resp.assert_status_ok();
    let json: serde_json::Value = resp.json();
    let tools = json["result"]["tools"].as_array().unwrap();
    assert_eq!(
        tools.len(),
        4,
        "Should have 4 tools: scrape, crawl, check, map"
    );

    let tool_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(tool_names.contains(&"crw_scrape"));
    assert!(tool_names.contains(&"crw_crawl"));
    assert!(tool_names.contains(&"crw_check_crawl_status"));
    assert!(tool_names.contains(&"crw_map"));
}

#[tokio::test]
async fn mcp_unknown_method_error_32601() {
    let server = test_app();
    let resp = server
        .post("/mcp")
        .content_type("application/json")
        .json(&mcp_request("nonexistent/method", json!(3), json!({})))
        .await;
    resp.assert_status_ok();
    let json: serde_json::Value = resp.json();
    assert_eq!(json["error"]["code"], -32601);
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("method not found")
    );
}

#[tokio::test]
async fn mcp_invalid_json_error_32700() {
    let server = test_app();
    let resp = server
        .post("/mcp")
        .content_type("application/json")
        .bytes("this is not valid json".into())
        .await;
    resp.assert_status_ok();
    let json: serde_json::Value = resp.json();
    assert_eq!(json["error"]["code"], -32700);
}

#[tokio::test]
async fn mcp_ping_returns_empty() {
    let server = test_app();
    let resp = server
        .post("/mcp")
        .content_type("application/json")
        .json(&mcp_request("ping", json!(4), json!({})))
        .await;
    resp.assert_status_ok();
    let json: serde_json::Value = resp.json();
    assert_eq!(json["result"], json!({}));
}

#[tokio::test]
async fn mcp_notification_returns_202() {
    let server = test_app();
    let body = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    let resp = server
        .post("/mcp")
        .content_type("application/json")
        .json(&body)
        .await;
    resp.assert_status(axum::http::StatusCode::ACCEPTED);
}

#[tokio::test]
async fn mcp_wrong_content_type_400() {
    let server = test_app();
    let resp = server
        .post("/mcp")
        .content_type("text/plain")
        .bytes(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#.into())
        .await;
    resp.assert_status(axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn mcp_tools_call_unknown_tool() {
    let server = test_app();
    let resp = server
        .post("/mcp")
        .content_type("application/json")
        .json(&mcp_request(
            "tools/call",
            json!(5),
            json!({"name": "nonexistent_tool", "arguments": {}}),
        ))
        .await;
    resp.assert_status_ok();
    let json: serde_json::Value = resp.json();
    let result = &json["result"];
    assert_eq!(result["isError"], true);
}

#[tokio::test]
async fn mcp_null_id() {
    let server = test_app();
    let resp = server
        .post("/mcp")
        .content_type("application/json")
        .json(&mcp_request("ping", json!(null), json!({})))
        .await;
    resp.assert_status_ok();
    let json: serde_json::Value = resp.json();
    assert!(json["id"].is_null());
}

#[tokio::test]
async fn mcp_integer_id() {
    let server = test_app();
    let resp = server
        .post("/mcp")
        .content_type("application/json")
        .json(&mcp_request("ping", json!(42), json!({})))
        .await;
    resp.assert_status_ok();
    let json: serde_json::Value = resp.json();
    assert_eq!(json["id"], 42);
}

#[tokio::test]
async fn mcp_missing_method_field() {
    let server = test_app();
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "params": {}
    });
    let resp = server
        .post("/mcp")
        .content_type("application/json")
        .json(&body)
        .await;
    resp.assert_status_ok();
    let json: serde_json::Value = resp.json();
    // Should be a parse error since "method" is required in JsonRpcRequest
    assert_eq!(json["error"]["code"], -32700);
}
