//! MCP (Model Context Protocol) stdio proxy for the CRW web scraper.
//!
//! This binary reads JSON-RPC requests from stdin and forwards them to a running
//! CRW server instance over HTTP. It exposes four tools to MCP-compatible AI clients:
//!
//! - `crw_scrape` — scrape a single URL
//! - `crw_crawl` — start an async BFS crawl
//! - `crw_check_crawl_status` — poll crawl job status
//! - `crw_map` — discover URLs on a website
//!
//! # Environment variables
//!
//! | Variable | Default | Description |
//! |----------|---------|-------------|
//! | `CRW_API_URL` | `http://localhost:3000` | CRW server base URL |
//! | `CRW_API_KEY` | *(none)* | Optional Bearer token for auth |
//!
//! # Usage
//!
//! ```bash
//! # Start with default settings
//! crw-mcp
//!
//! # Point to a remote server
//! CRW_API_URL=https://crw.example.com crw-mcp
//! ```

use crw_core::mcp::{
    JsonRpcRequest, JsonRpcResponse, ProtocolResult, handle_protocol_method, tool_result_response,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const SERVER_NAME: &str = "crw-mcp";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

// --- HTTP dispatch ---

async fn call_tool(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &Option<String>,
    tool_name: &str,
    args: Value,
) -> Result<Value, String> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("content-type", "application/json".parse().unwrap());
    if let Some(key) = api_key {
        headers.insert(
            "authorization",
            format!("Bearer {key}")
                .parse()
                .map_err(|e| format!("invalid api key: {e}"))?,
        );
    }

    match tool_name {
        "crw_scrape" => {
            let resp = client
                .post(format!("{base_url}/v1/scrape"))
                .headers(headers)
                .json(&args)
                .send()
                .await
                .map_err(|e| format!("HTTP request failed: {e}"))?;
            parse_response(resp).await
        }
        "crw_crawl" => {
            let resp = client
                .post(format!("{base_url}/v1/crawl"))
                .headers(headers)
                .json(&args)
                .send()
                .await
                .map_err(|e| format!("HTTP request failed: {e}"))?;
            parse_response(resp).await
        }
        "crw_check_crawl_status" => {
            let id = args
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("missing required parameter: id")?;
            let resp = client
                .get(format!("{base_url}/v1/crawl/{id}"))
                .headers(headers)
                .send()
                .await
                .map_err(|e| format!("HTTP request failed: {e}"))?;
            parse_response(resp).await
        }
        "crw_map" => {
            let resp = client
                .post(format!("{base_url}/v1/map"))
                .headers(headers)
                .json(&args)
                .send()
                .await
                .map_err(|e| format!("HTTP request failed: {e}"))?;
            parse_response(resp).await
        }
        _ => Err(format!("unknown tool: {tool_name}")),
    }
}

async fn parse_response(resp: reqwest::Response) -> Result<Value, String> {
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("failed to read response: {e}"))?;

    if !status.is_success() {
        return Err(format!("API error ({}): {}", status, truncate(&body, 500)));
    }

    serde_json::from_str(&body).map_err(|e| format!("invalid JSON response: {e}"))
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        // Find the last char boundary at or before `max` to avoid UTF-8 panic.
        let end = s.floor_char_boundary(max);
        &s[..end]
    }
}

// --- Request handling ---

async fn handle_request(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &Option<String>,
    req: JsonRpcRequest,
) -> Option<JsonRpcResponse> {
    // Handle common protocol methods via shared logic.
    match handle_protocol_method(SERVER_NAME, SERVER_VERSION, &req) {
        ProtocolResult::Response(resp) => return Some(resp),
        ProtocolResult::Notification => return None,
        ProtocolResult::NotHandled => {}
    }

    // Only remaining method: tools/call
    match req.method.as_str() {
        "tools/call" => {
            let id = req.id.unwrap_or(Value::Null);
            let tool_name = req
                .params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let arguments = req.params.get("arguments").cloned().unwrap_or(json!({}));

            let result = call_tool(client, base_url, api_key, tool_name, arguments).await;
            Some(tool_result_response(id, result))
        }

        _ => {
            if let Some(id) = req.id {
                Some(JsonRpcResponse::error(
                    id,
                    -32601,
                    format!("method not found: {}", req.method),
                ))
            } else {
                None
            }
        }
    }
}

// --- Main ---

#[tokio::main]
async fn main() {
    // Log to stderr so stdout stays clean for MCP protocol
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "crw_mcp=info".parse().unwrap()),
        )
        .init();

    let base_url = std::env::var("CRW_API_URL").unwrap_or_else(|_| "http://localhost:3000".into());
    let api_key = std::env::var("CRW_API_KEY").ok();

    tracing::info!("Starting {SERVER_NAME} v{SERVER_VERSION}");
    tracing::info!("API URL: {base_url}");

    let client = reqwest::Client::builder()
        .redirect(crw_core::url_safety::safe_redirect_policy())
        .timeout(std::time::Duration::from_secs(120))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("reqwest client build failed");
    let mut stdout = tokio::io::stdout();

    // Use tokio async stdin to avoid blocking the async runtime.
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(e) => {
                tracing::error!("stdin read error: {e}");
                break;
            }
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        tracing::debug!("← {trimmed}");

        let req: JsonRpcRequest = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                let err = JsonRpcResponse::error(Value::Null, -32700, format!("parse error: {e}"));
                let out = serde_json::to_string(&err).unwrap();
                tracing::debug!("→ {out}");
                let _ = stdout.write_all(out.as_bytes()).await;
                let _ = stdout.write_all(b"\n").await;
                let _ = stdout.flush().await;
                continue;
            }
        };

        if let Some(resp) = handle_request(&client, &base_url, &api_key, req).await {
            let out = serde_json::to_string(&resp).unwrap();
            tracing::debug!("→ {out}");
            let _ = stdout.write_all(out.as_bytes()).await;
            let _ = stdout.write_all(b"\n").await;
            let _ = stdout.flush().await;
        }
    }
}
