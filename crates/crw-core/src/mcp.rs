//! Shared MCP (Model Context Protocol) JSON-RPC types and tool definitions.
//!
//! Used by both the HTTP MCP endpoint (`crw-server`) and the stdio MCP proxy (`crw-mcp`).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const PROTOCOL_VERSION: &str = "2024-11-05";

// --- JSON-RPC types ---

#[derive(Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Serialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

impl JsonRpcResponse {
    pub fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Value, code: i64, message: String) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(JsonRpcError { code, message }),
        }
    }
}

// --- Tool definitions ---

pub fn tool_definitions(proxy_mode: bool) -> Value {
    let mut tools = vec![
        json!({
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
        }),
        json!({
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
        }),
        json!({
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
        }),
        json!({
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
        }),
    ];

    if proxy_mode {
        tools.push(json!({
            "name": "crw_search",
            "description": "Search the web and return relevant results with titles, URLs, and descriptions. Powered by fastCRW cloud — only available in proxy mode.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of results to return (default: 5)"
                    },
                    "lang": {
                        "type": "string",
                        "description": "Language code for results (e.g. \"en\", \"tr\")"
                    },
                    "country": {
                        "type": "string",
                        "description": "Country code for results (e.g. \"us\", \"tr\")"
                    },
                    "scrapeOptions": {
                        "type": "object",
                        "description": "Options for scraping each result page (e.g. {\"formats\": [\"markdown\"]})",
                        "properties": {
                            "formats": {
                                "type": "array",
                                "items": { "type": "string", "enum": ["markdown", "html", "links"] }
                            }
                        }
                    }
                },
                "required": ["query"]
            }
        }));
    }

    json!({ "tools": tools })
}

/// Result of handling a protocol method.
pub enum ProtocolResult {
    /// Send this response back to the client.
    Response(JsonRpcResponse),
    /// Notification — no response needed.
    Notification,
    /// Not a protocol method — caller should handle it.
    NotHandled,
}

/// Handle common MCP protocol methods (initialize, tools/list, ping, notifications).
pub fn handle_protocol_method(
    server_name: &str,
    server_version: &str,
    req: &JsonRpcRequest,
    proxy_mode: bool,
) -> ProtocolResult {
    if req.jsonrpc != "2.0" {
        let id = req.id.clone().unwrap_or(Value::Null);
        return ProtocolResult::Response(JsonRpcResponse::error(
            id,
            -32600,
            "invalid jsonrpc version".into(),
        ));
    }

    match req.method.as_str() {
        "notifications/initialized" | "notifications/cancelled" => ProtocolResult::Notification,

        "initialize" => {
            let id = req.id.clone().unwrap_or(Value::Null);
            ProtocolResult::Response(JsonRpcResponse::success(
                id,
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": { "tools": {} },
                    "serverInfo": {
                        "name": server_name,
                        "version": server_version
                    }
                }),
            ))
        }

        "tools/list" => {
            let id = req.id.clone().unwrap_or(Value::Null);
            ProtocolResult::Response(JsonRpcResponse::success(id, tool_definitions(proxy_mode)))
        }

        "ping" => {
            let id = req.id.clone().unwrap_or(Value::Null);
            ProtocolResult::Response(JsonRpcResponse::success(id, json!({})))
        }

        _ => ProtocolResult::NotHandled,
    }
}

/// Wrap a tool call result into an MCP-compliant content response.
pub fn tool_result_response(id: Value, result: Result<Value, String>) -> JsonRpcResponse {
    match result {
        Ok(value) => {
            let text = serde_json::to_string_pretty(&value).unwrap_or_default();
            JsonRpcResponse::success(
                id,
                json!({
                    "content": [{"type": "text", "text": text}]
                }),
            )
        }
        Err(e) => JsonRpcResponse::success(
            id,
            json!({
                "content": [{"type": "text", "text": e}],
                "isError": true
            }),
        ),
    }
}
