use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use crw_core::types::{CrawlRequest, MapRequest, ScrapeRequest};
use crw_crawl::crawl::discover_urls;
use crw_crawl::single::scrape_url;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Instant;
use uuid::Uuid;

use crate::state::{AppState, CrawlJob};

const SERVER_NAME: &str = "crw";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const PROTOCOL_VERSION: &str = "2024-11-05";

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
                            "description": "CSS selectors to include"
                        },
                        "excludeTags": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "CSS selectors to exclude"
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
                        "url": { "type": "string", "description": "The starting URL to crawl" },
                        "maxDepth": { "type": "integer", "description": "Maximum crawl depth (default: 2)" },
                        "maxPages": { "type": "integer", "description": "Maximum number of pages to crawl (default: 10)" }
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
                        "id": { "type": "string", "description": "The crawl job ID returned by crw_crawl" }
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
                        "url": { "type": "string", "description": "The URL to map" },
                        "maxDepth": { "type": "integer", "description": "Maximum crawl depth for discovery (default: 2)" },
                        "useSitemap": { "type": "boolean", "description": "Whether to use the site's sitemap.xml (default: true)" }
                    },
                    "required": ["url"]
                }
            }
        ]
    })
}

/// Validate URL safety for MCP tool calls (same checks as REST API routes).
fn validate_url(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("Invalid URL: {e}"))?;
    crw_core::url_safety::validate_safe_url(&parsed)
}

async fn call_tool(state: &AppState, tool_name: &str, args: Value) -> Result<Value, String> {
    match tool_name {
        "crw_scrape" => {
            let req: ScrapeRequest =
                serde_json::from_value(args).map_err(|e| format!("invalid arguments: {e}"))?;
            validate_url(&req.url)?;
            let llm_config = state.config.extraction.llm.as_ref();
            let data = scrape_url(&req, &state.renderer, llm_config)
                .await
                .map_err(|e| format!("{e}"))?;
            serde_json::to_value(&data).map_err(|e| format!("serialize error: {e}"))
        }
        "crw_crawl" => {
            let req: CrawlRequest =
                serde_json::from_value(args).map_err(|e| format!("invalid arguments: {e}"))?;
            validate_url(&req.url)?;
            let id = Uuid::new_v4();
            let initial = crw_core::types::CrawlState {
                id,
                status: crw_core::types::CrawlStatus::InProgress,
                total: 0,
                completed: 0,
                data: vec![],
                error: None,
            };
            let (tx, rx) = tokio::sync::watch::channel(initial);
            {
                let mut jobs = state.crawl_jobs.write().await;
                jobs.insert(
                    id,
                    CrawlJob {
                        rx,
                        created_at: Instant::now(),
                    },
                );
            }
            let renderer = state.renderer.clone();
            let max_concurrency = state.config.crawler.max_concurrency;
            let respect_robots = state.config.crawler.respect_robots_txt;
            let rps = state.config.crawler.requests_per_second;
            let user_agent = state.config.crawler.user_agent.clone();
            let crawl_semaphore = state.crawl_semaphore.clone();
            let llm_config = state.config.extraction.llm.clone();
            tokio::spawn(async move {
                let _permit = crawl_semaphore.acquire().await;
                crw_crawl::crawl::run_crawl(
                    id,
                    req,
                    renderer,
                    max_concurrency,
                    respect_robots,
                    rps,
                    &user_agent,
                    tx,
                    llm_config.as_ref(),
                )
                .await;
            });
            Ok(json!({"success": true, "id": id.to_string()}))
        }
        "crw_check_crawl_status" => {
            let id_str = args
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("missing required parameter: id")?;
            let id: Uuid = id_str
                .parse()
                .map_err(|_| format!("invalid crawl id: {id_str}"))?;
            let jobs = state.crawl_jobs.read().await;
            let job = jobs.get(&id).ok_or(format!("crawl job {id} not found"))?;
            let current = job.rx.borrow().clone();
            serde_json::to_value(&current).map_err(|e| format!("serialize error: {e}"))
        }
        "crw_map" => {
            let req: MapRequest =
                serde_json::from_value(args).map_err(|e| format!("invalid arguments: {e}"))?;
            validate_url(&req.url)?;
            let max_depth = req
                .max_depth
                .unwrap_or(state.config.crawler.default_max_depth);
            let urls = discover_urls(
                &req.url,
                max_depth,
                req.use_sitemap,
                &state.renderer,
                state.config.crawler.max_concurrency,
                state.config.crawler.requests_per_second,
                &state.config.crawler.user_agent,
            )
            .await
            .map_err(|e| format!("{e}"))?;
            Ok(json!({"success": true, "links": urls}))
        }
        _ => Err(format!("unknown tool: {tool_name}")),
    }
}

async fn handle_request(state: &AppState, req: JsonRpcRequest) -> Option<JsonRpcResponse> {
    if req.jsonrpc != "2.0" {
        if let Some(id) = req.id {
            return Some(JsonRpcResponse::error(
                id,
                -32600,
                "invalid jsonrpc version".into(),
            ));
        }
        return None;
    }

    match req.method.as_str() {
        "notifications/initialized" | "notifications/cancelled" => None,

        "initialize" => {
            let id = req.id.unwrap_or(Value::Null);
            Some(JsonRpcResponse::success(
                id,
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": { "tools": {} },
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
            let arguments = req.params.get("arguments").cloned().unwrap_or(json!({}));

            match call_tool(state, tool_name, arguments).await {
                Ok(result) => {
                    let text = serde_json::to_string_pretty(&result).unwrap_or_default();
                    Some(JsonRpcResponse::success(
                        id,
                        json!({
                            "content": [{"type": "text", "text": text}]
                        }),
                    ))
                }
                Err(e) => Some(JsonRpcResponse::success(
                    id,
                    json!({
                        "content": [{"type": "text", "text": e}],
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

/// POST /mcp — Streamable HTTP MCP transport.
/// Handles JSON-RPC 2.0 requests over HTTP POST.
pub async fn mcp_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    // Validate content type
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !content_type.contains("application/json") {
        return (
            StatusCode::BAD_REQUEST,
            [("content-type", "application/json")],
            serde_json::to_string(&JsonRpcResponse::error(
                Value::Null,
                -32700,
                "Content-Type must be application/json".into(),
            ))
            .unwrap(),
        );
    }

    let req: JsonRpcRequest = match serde_json::from_str(&body) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::OK,
                [("content-type", "application/json")],
                serde_json::to_string(&JsonRpcResponse::error(
                    Value::Null,
                    -32700,
                    format!("parse error: {e}"),
                ))
                .unwrap(),
            );
        }
    };

    match handle_request(&state, req).await {
        Some(resp) => (
            StatusCode::OK,
            [("content-type", "application/json")],
            serde_json::to_string(&resp).unwrap(),
        ),
        // Notification — no response body, return 202 Accepted
        None => (
            StatusCode::ACCEPTED,
            [("content-type", "application/json")],
            String::new(),
        ),
    }
}
