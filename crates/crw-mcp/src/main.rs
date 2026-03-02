use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::BufRead;
use tokio::io::AsyncWriteExt;

const SERVER_NAME: &str = "crw-mcp";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const PROTOCOL_VERSION: &str = "2024-11-05";

// --- JSON-RPC types ---

#[derive(Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

impl JsonRpcResponse {
    fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Value, code: i64, message: String) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(JsonRpcError { code, message }),
        }
    }
}

// --- Tool definitions ---

fn tool_definitions() -> Value {
    json!({
        "tools": [
            {
                "name": "crw_scrape",
                "description": "Scrape a single URL and return its content as markdown, HTML, or links. Use this to extract content from any web page.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "The URL to scrape"
                        },
                        "formats": {
                            "type": "array",
                            "items": { "type": "string", "enum": ["markdown", "html", "links"] },
                            "description": "Output formats (default: [\"markdown\"])"
                        },
                        "onlyMainContent": {
                            "type": "boolean",
                            "description": "Extract only the main content, removing nav/footer/etc (default: true)"
                        },
                        "includeTags": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "CSS selectors to include (only content matching these selectors)"
                        },
                        "excludeTags": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "CSS selectors to exclude from output"
                        }
                    },
                    "required": ["url"]
                }
            },
            {
                "name": "crw_crawl",
                "description": "Start an async crawl of a website. Returns a job ID that can be polled with crw_check_crawl_status.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "The starting URL to crawl"
                        },
                        "maxDepth": {
                            "type": "integer",
                            "description": "Maximum crawl depth (default: 2)"
                        },
                        "maxPages": {
                            "type": "integer",
                            "description": "Maximum number of pages to crawl (default: 10)"
                        },
                        "jsonSchema": {
                            "type": "object",
                            "description": "JSON schema for LLM-based structured data extraction on each crawled page"
                        }
                    },
                    "required": ["url"]
                }
            },
            {
                "name": "crw_check_crawl_status",
                "description": "Check the status of an async crawl job and retrieve results.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "The crawl job ID returned by crw_crawl"
                        }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "crw_map",
                "description": "Discover URLs on a website by crawling and/or reading its sitemap.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "The URL to map"
                        },
                        "maxDepth": {
                            "type": "integer",
                            "description": "Maximum crawl depth for discovery (default: 2)"
                        },
                        "useSitemap": {
                            "type": "boolean",
                            "description": "Whether to use the site's sitemap.xml (default: true)"
                        }
                    },
                    "required": ["url"]
                }
            }
        ]
    })
}

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
            format!("Bearer {key}").parse().map_err(|e| format!("invalid api key: {e}"))?,
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
        &s[..max]
    }
}

// --- Request handling ---

async fn handle_request(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &Option<String>,
    req: JsonRpcRequest,
) -> Option<JsonRpcResponse> {
    if req.jsonrpc != "2.0" {
        if let Some(id) = req.id {
            return Some(JsonRpcResponse::error(id, -32600, "invalid jsonrpc version".into()));
        }
        return None;
    }

    match req.method.as_str() {
        // Notifications (no id = no response)
        "notifications/initialized" | "notifications/cancelled" => None,

        "initialize" => {
            let id = req.id.unwrap_or(Value::Null);
            Some(JsonRpcResponse::success(
                id,
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": SERVER_NAME,
                        "version": SERVER_VERSION
                    }
                }),
            ))
        }

        "tools/list" => {
            let id = req.id.unwrap_or(Value::Null);
            Some(JsonRpcResponse::success(id, tool_definitions()))
        }

        "tools/call" => {
            let id = req.id.unwrap_or(Value::Null);
            let tool_name = req
                .params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let arguments = req
                .params
                .get("arguments")
                .cloned()
                .unwrap_or(json!({}));

            match call_tool(client, base_url, api_key, tool_name, arguments).await {
                Ok(result) => {
                    let text = serde_json::to_string_pretty(&result).unwrap_or_default();
                    Some(JsonRpcResponse::success(
                        id,
                        json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": text
                                }
                            ]
                        }),
                    ))
                }
                Err(e) => Some(JsonRpcResponse::success(
                    id,
                    json!({
                        "content": [
                            {
                                "type": "text",
                                "text": e
                            }
                        ],
                        "isError": true
                    }),
                )),
            }
        }

        "ping" => {
            let id = req.id.unwrap_or(Value::Null);
            Some(JsonRpcResponse::success(id, json!({})))
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

    let base_url = std::env::var("CRW_API_URL")
        .unwrap_or_else(|_| "http://localhost:3000".into());
    let api_key = std::env::var("CRW_API_KEY").ok();

    tracing::info!("Starting {SERVER_NAME} v{SERVER_VERSION}");
    tracing::info!("API URL: {base_url}");

    let client = reqwest::Client::new();
    let mut stdout = tokio::io::stdout();

    let stdin = std::io::stdin();
    let reader = stdin.lock();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                tracing::error!("stdin read error: {e}");
                break;
            }
        };

        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        tracing::debug!("← {line}");

        let req: JsonRpcRequest = match serde_json::from_str(&line) {
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
