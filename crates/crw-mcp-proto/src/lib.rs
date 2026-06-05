//! Shared MCP (Model Context Protocol) JSON-RPC types and tool definitions.
//!
//! Used by both the HTTP MCP endpoint (`crw-server`) and the stdio MCP proxy (`crw-mcp`).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// MCP spec revision advertised in the `initialize` handshake (lib.rs `initialize`
/// arm). Bumped from "2024-11-05" to "2025-06-18" to legitimize tool `outputSchema`
/// and result `structuredContent`, both introduced in the 2025-06-18 revision.
/// There is no per-feature capability flag for structured output, so advertising
/// the revision that defines it is the only spec-legal way to emit it.
///
/// NOTE: `crw-browse` is a separate rmcp-based MCP server that pins its own
/// `ProtocolVersion::V_2024_11_05` (crw-browse/src/server.rs) and does NOT consume
/// this constant — it intentionally stays on 2024-11-05.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

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
                    },
                    "renderJs": {
                        "type": "boolean",
                        "description": "Render JavaScript before extracting (true = force JS, false = HTTP only, omit = auto-detect or use the server's render_js_default)"
                    },
                    "waitFor": {
                        "type": "integer",
                        "description": "Milliseconds to wait after JS rendering for late content/XHRs"
                    },
                    "renderer": {
                        "type": "string",
                        "enum": ["auto", "lightpanda", "chrome", "playwright"],
                        "description": "Pin this request to a specific renderer. \"auto\" (default if omitted) uses the configured fallback chain. Other values hard-pin to a single renderer with no fallback. Pinning a non-auto value implies renderJs:true unless renderJs:false is set explicitly."
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
                    },
                    "renderJs": {
                        "type": "boolean",
                        "description": "Render JavaScript on every crawled page (true = force JS, false = HTTP only, omit = auto-detect or use the server's render_js_default)"
                    },
                    "waitFor": {
                        "type": "integer",
                        "description": "Milliseconds to wait after JS rendering on each page"
                    },
                    "renderer": {
                        "type": "string",
                        "enum": ["auto", "lightpanda", "chrome", "playwright"],
                        "description": "Pin every crawled page to a specific renderer. \"auto\" (default if omitted) uses the configured fallback chain. Other values hard-pin with no fallback. Pinning a non-auto value implies renderJs:true unless renderJs:false is set explicitly."
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
                    },
                    "crawlFallback": {
                        "type": "boolean",
                        "description": "If true (default), supplements sitemap discovery with a short BFS crawl when the sitemap returns enough URLs. Set false for sitemap-only mode (faster, may miss pages not in the sitemap)."
                    }
                },
                "required": ["url"]
            }
        }),
    ];

    // `crw_search` is always advertised. In embedded mode it dispatches to a
    // local SearXNG sidecar via crw-server's `/v1/search` pipeline; in proxy
    // mode it forwards to the configured remote API. Whether the underlying
    // SearXNG instance is configured is a runtime concern — the server returns
    // a clear `search_disabled` error when [search].searxng_url is unset.
    let _ = proxy_mode;
    tools.push(json!({
        "name": "crw_search",
        "description": "Search the web and return relevant results with titles, URLs, and descriptions/snippets. Backed by a SearXNG sidecar in embedded mode (no API key needed), or by the configured remote API in proxy mode (uses CRW_API_KEY).\n\nReturn shape: `{ \"success\": true, \"data\": { \"results\": [{ \"url\", \"title\", \"description\", \"snippet\", \"position\", \"score\" }, ...] } }`. When `sources` is set, `data.results` is instead an object grouped by source (`{ \"web\": [...], \"news\": [...], \"images\": [...] }`). The `snippet` field is an alias of `description` — both carry the same body text so downstream LLM pipelines that ask for either get a match.\n\nExample: `crw_search(query=\"renewable energy trends 2024\", limit=3)` returns the top 3 web results with title/url/snippet.\n\nErrors: returns `search_disabled` when no SearXNG backend is configured, or `target_unreachable` / `timeout` (naming the configured host) when the backend can't be reached.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 5, max: 20)"
                },
                "lang": {
                    "type": "string",
                    "description": "Language code for results (e.g. \"en\", \"tr\")"
                },
                "country": {
                    "type": "string",
                    "description": "Country code for results (e.g. \"us\", \"tr\"). Hint to bias regional results; ignored if the underlying engine does not support it."
                },
                "tbs": {
                    "type": "string",
                    "enum": ["qdr:h", "qdr:d", "qdr:w", "qdr:m", "qdr:y"],
                    "description": "Time filter — restrict to results from the past hour/day/week/month/year"
                },
                "sources": {
                    "type": "array",
                    "items": { "type": "string", "enum": ["web", "news", "images"] },
                    "description": "If set, returns results grouped by source instead of a flat list"
                },
                "categories": {
                    "type": "array",
                    "items": { "type": "string", "enum": ["github", "research", "pdf"] },
                    "description": "Bias the search towards a category. `pdf` appends `filetype:pdf` to the query; `github`/`research` switch to topical engines."
                },
                "scrapeOptions": {
                    "type": "object",
                    "description": "If set, each `web` result is scraped in-process and the requested formats are inlined into the response.",
                    "properties": {
                        "formats": {
                            "type": "array",
                            "items": { "type": "string", "enum": ["markdown", "html", "rawHtml", "links"] }
                        },
                        "onlyMainContent": {
                            "type": "boolean",
                            "description": "Strip nav/footer/ads before serializing (default: true)"
                        }
                    }
                }
            },
            "required": ["query"]
        },
        // Mirrors the real `SearchResponse = ApiResponse<SearchResponseData>`
        // serializer in crw-core/src/types.rs. The Ok branch of
        // `tool_result_response` is the only path that emits `structuredContent`,
        // and `ApiResponse::ok` always sets `data: Some(..)` — so `data` is
        // required and the error/error_code/warning siblings are never present
        // on the validated value (they only appear on the err() path, which
        // returns Err(String) and never reaches structuredContent).
        //
        // `data.results` is `#[serde(untagged)] enum SearchData` → a flat array
        // (no `sources`) OR a grouped object (`web`/`news`/`images`). Hence the
        // `oneOf`. Grouped `images` deserialize to `ImageResult` (carries
        // `imageUrl`, NO `snippet`), so they are deliberately left unconstrained —
        // constraining them with the text-result shape would falsely reject every
        // real grouped-image response. No `additionalProperties: false` anywhere:
        // SearchResult conditionally emits markdown/html/links/metadata/etc.
        "outputSchema": {
            "type": "object",
            "$defs": {
                "searchResultItem": {
                    "type": "object",
                    "properties": {
                        "url": { "type": "string" },
                        "title": { "type": "string" },
                        "description": { "type": "string", "description": "Body snippet for the result. `snippet` is an alias of this field." },
                        "snippet": { "type": "string", "description": "Alias of `description`. Always populated." },
                        "position": { "type": "integer" },
                        "score": { "type": "number" },
                        "category": { "type": "string" }
                    },
                    "required": ["url", "title", "description", "snippet", "position"]
                }
            },
            "properties": {
                "success": { "type": "boolean" },
                "data": {
                    "type": "object",
                    "properties": {
                        "results": {
                            "oneOf": [
                                { "type": "array", "items": { "$ref": "#/$defs/searchResultItem" } },
                                {
                                    "type": "object",
                                    "properties": {
                                        "web": { "type": "array", "items": { "$ref": "#/$defs/searchResultItem" } },
                                        "news": { "type": "array", "items": { "$ref": "#/$defs/searchResultItem" } },
                                        "images": { "type": "array" }
                                    }
                                }
                            ]
                        },
                        "answer": { "type": "string" },
                        "citations": { "type": "array" },
                        "llmUsage": { "type": "object" },
                        "warnings": { "type": "array", "items": { "type": "string" } }
                    },
                    "required": ["results"]
                },
                "error": { "type": "string" },
                "error_code": { "type": "string" },
                "warning": { "type": "string" }
            },
            "required": ["success", "data"]
        }
    }));

    json!({ "tools": tools })
}

/// Returns the declared `outputSchema` for a tool, if it declares one.
///
/// Single source of truth: `structuredContent` emission is derived from the
/// same `tool_definitions` declaration that `tools/list` advertises, so the two
/// can never drift. Recomputes `tool_definitions` per call — `tools/call` is not
/// hot; memoize behind a `OnceLock` only if profiling ever demands it.
pub fn tool_output_schema(tool_name: &str) -> Option<Value> {
    tool_definitions(false)["tools"]
        .as_array()?
        .iter()
        .find(|t| t["name"] == tool_name)
        .and_then(|t| t.get("outputSchema").cloned())
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
///
/// On success the structured `value` is emitted **both** as a text content block
/// (verbatim, for backward compatibility with lenient clients and clients that
/// negotiated an older protocol revision) **and**, when the called tool declares
/// an `outputSchema`, as a top-level `structuredContent` field (MCP 2025-06-18)
/// so strict clients can validate it. Both representations derive from the same
/// `value` binding, so `serde_json::from_str(content[0].text) == structuredContent`
/// holds by construction — the two can never disagree.
pub fn tool_result_response(
    id: Value,
    tool_name: &str,
    result: Result<Value, String>,
) -> JsonRpcResponse {
    match result {
        Ok(value) => {
            let text = serde_json::to_string_pretty(&value).unwrap_or_default();
            let mut payload = json!({
                "content": [{"type": "text", "text": text}]
            });
            // Attach structuredContent only when (a) the tool declares an
            // outputSchema and (b) the value is a JSON object — the spec requires
            // structuredContent to be an object. The `is_object()` guard is the
            // proxy version-skew safety valve: in proxy mode a schema-bearing tool
            // may yield a non-object Ok value (an upstream error string, a plain
            // string, or a legacy top-level array) — degrade to text-only rather
            // than ship a spec-violating structuredContent to a strict client.
            // Locked by test T2b. Do NOT remove the is_object() guard.
            if value.is_object() && tool_output_schema(tool_name).is_some() {
                payload["structuredContent"] = value;
            }
            JsonRpcResponse::success(id, payload)
        }
        // Err path: never attach structuredContent. `isError:true` signals
        // failure, and strict clients must not validate outputSchema against an
        // error result.
        Err(e) => JsonRpcResponse::success(
            id,
            json!({
                "content": [{"type": "text", "text": e}],
                "isError": true
            }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_by_name<'a>(tools: &'a Value, name: &str) -> &'a Value {
        tools["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("tool {name} not found"))
    }

    #[test]
    fn crw_scrape_schema_advertises_render_js() {
        let defs = tool_definitions(false);
        let scrape = tool_by_name(&defs, "crw_scrape");
        let props = &scrape["inputSchema"]["properties"];
        assert_eq!(
            props["renderJs"]["type"], "boolean",
            "renderJs must be a plain boolean in the advertised schema"
        );
        assert!(
            props["renderJs"].get("default").is_none(),
            "renderJs must not advertise a default — server resolves it"
        );
    }

    #[test]
    fn crw_scrape_schema_advertises_wait_for() {
        let defs = tool_definitions(false);
        let scrape = tool_by_name(&defs, "crw_scrape");
        let props = &scrape["inputSchema"]["properties"];
        assert_eq!(props["waitFor"]["type"], "integer");
    }

    #[test]
    fn crw_scrape_render_js_not_required() {
        let defs = tool_definitions(false);
        let scrape = tool_by_name(&defs, "crw_scrape");
        let required = scrape["inputSchema"]["required"]
            .as_array()
            .expect("required array");
        assert!(
            !required.iter().any(|v| v == "renderJs"),
            "renderJs must not be in required"
        );
        assert!(
            !required.iter().any(|v| v == "waitFor"),
            "waitFor must not be in required"
        );
    }

    #[test]
    fn crw_crawl_schema_advertises_render_js_and_wait_for() {
        let defs = tool_definitions(false);
        let crawl = tool_by_name(&defs, "crw_crawl");
        let props = &crawl["inputSchema"]["properties"];
        assert_eq!(props["renderJs"]["type"], "boolean");
        assert_eq!(props["waitFor"]["type"], "integer");
    }

    #[test]
    fn crw_scrape_schema_advertises_renderer() {
        let defs = tool_definitions(false);
        let scrape = tool_by_name(&defs, "crw_scrape");
        let props = &scrape["inputSchema"]["properties"];
        assert_eq!(props["renderer"]["type"], "string");
        let enum_vals = props["renderer"]["enum"]
            .as_array()
            .expect("renderer.enum must be an array");
        assert_eq!(
            enum_vals,
            &vec![
                json!("auto"),
                json!("lightpanda"),
                json!("chrome"),
                json!("playwright"),
            ]
        );
    }

    #[test]
    fn crw_scrape_renderer_not_required() {
        let defs = tool_definitions(false);
        let scrape = tool_by_name(&defs, "crw_scrape");
        let required = scrape["inputSchema"]["required"]
            .as_array()
            .expect("required array");
        assert!(!required.iter().any(|v| v == "renderer"));
    }

    #[test]
    fn crw_crawl_schema_advertises_renderer() {
        let defs = tool_definitions(false);
        let crawl = tool_by_name(&defs, "crw_crawl");
        let props = &crawl["inputSchema"]["properties"];
        assert_eq!(props["renderer"]["type"], "string");
        let enum_vals = props["renderer"]["enum"]
            .as_array()
            .expect("renderer.enum must be an array");
        assert_eq!(enum_vals.len(), 4);
        assert!(enum_vals.iter().any(|v| v == "chrome"));
        assert!(enum_vals.iter().any(|v| v == "lightpanda"));
        assert!(enum_vals.iter().any(|v| v == "auto"));
        assert!(enum_vals.iter().any(|v| v == "playwright"));
    }

    #[test]
    fn schemas_do_not_set_additional_properties_false() {
        // Deferred to a follow-up issue. Guard against accidentally enabling
        // this before the schemas are expanded to full ScrapeRequest parity.
        let defs = tool_definitions(false);
        for name in ["crw_scrape", "crw_crawl", "crw_map"] {
            let tool = tool_by_name(&defs, name);
            let ap = &tool["inputSchema"].get("additionalProperties");
            assert!(
                ap.is_none() || ap.as_ref().and_then(|v| v.as_bool()) != Some(false),
                "{name}: additionalProperties:false must remain off until schemas are complete"
            );
        }
    }

    // --- structuredContent emission (issue #89) ---

    /// A single text-result item with every always-emitted field set, plus the
    /// optional `score`/`category`. `snippet` mirrors `description`, matching the
    /// real `SearchResult` serializer (snippet is an alias of description).
    fn search_result_item(idx: u32) -> Value {
        json!({
            "url": format!("https://example.com/{idx}"),
            "title": format!("Result {idx}"),
            "description": "body text",
            "snippet": "body text",
            "position": idx,
            "score": 4.0,
            "category": "general"
        })
    }

    /// A representative flat (`sources` unset) crw_search success value, shaped
    /// like `ApiResponse::ok(SearchResponseData { results: Flat(..), .. })`.
    fn representative_search_value() -> Value {
        json!({
            "success": true,
            "data": { "results": [search_result_item(1), search_result_item(2)] }
        })
    }

    /// A representative grouped (`sources` set) value: `results` is an object with
    /// `web`/`news` (text items) and `images` (the differently-shaped ImageResult).
    fn grouped_search_value() -> Value {
        json!({
            "success": true,
            "data": { "results": {
                "web": [search_result_item(1)],
                "news": [search_result_item(2)],
                "images": [{
                    "url": "https://example.com/img",
                    "title": "An image",
                    "description": "alt text",
                    "imageUrl": "https://example.com/img.png",
                    "position": 1
                }]
            }}
        })
    }

    fn result_of(resp: &JsonRpcResponse) -> &Value {
        resp.result.as_ref().expect("success response has result")
    }

    /// T1 — crw_search Ok emits BOTH a text block and structuredContent, and the
    /// two are byte-for-byte the same value (single-source invariant).
    #[test]
    fn t1_search_emits_dual_content_in_sync() {
        let repr = representative_search_value();
        let resp = tool_result_response(json!(1), "crw_search", Ok(repr.clone()));
        let result = result_of(&resp);

        let text = result["content"][0]["text"]
            .as_str()
            .expect("text content present");
        assert_eq!(
            result["content"][0]["type"], "text",
            "first content block is text"
        );

        let structured = &result["structuredContent"];
        assert!(!structured.is_null(), "structuredContent present");
        assert_eq!(
            structured, &repr,
            "structuredContent is the unmodified value"
        );

        let from_text: Value = serde_json::from_str(text).expect("text is valid JSON");
        assert_eq!(
            &from_text, structured,
            "from_str(content.text) == structuredContent (no drift)"
        );
    }

    /// T2 — a tool WITHOUT an outputSchema (crw_scrape) gets text only, no
    /// structuredContent (schema-gated emission).
    #[test]
    fn t2_scrape_has_no_structured_content() {
        let resp = tool_result_response(json!(1), "crw_scrape", Ok(json!({"markdown": "hi"})));
        let result = result_of(&resp);
        assert!(result["content"][0]["text"].is_string());
        assert!(
            result.get("structuredContent").is_none(),
            "crw_scrape declares no outputSchema → no structuredContent"
        );
    }

    /// T2b — proxy version-skew safety valve: a schema-bearing tool whose Ok
    /// value is NOT an object (upstream error string, or a legacy top-level
    /// array) degrades to text-only. Locks the is_object() guard.
    #[test]
    fn t2b_non_object_search_value_degrades_to_text() {
        for non_object in [json!("upstream error string"), json!([{ "url": "x" }])] {
            let resp = tool_result_response(json!(1), "crw_search", Ok(non_object.clone()));
            let result = result_of(&resp);
            assert!(
                result["content"][0]["text"].is_string(),
                "text block carries the body"
            );
            assert!(
                result.get("structuredContent").is_none(),
                "non-object Ok value must NOT emit structuredContent: {non_object}"
            );
        }
    }

    /// T3 — the Err path is an isError text result with no structuredContent.
    #[test]
    fn t3_error_path_has_no_structured_content() {
        let resp = tool_result_response(json!(1), "crw_search", Err("boom".into()));
        let result = result_of(&resp);
        assert_eq!(result["isError"], true);
        assert_eq!(result["content"][0]["text"], "boom");
        assert!(result.get("structuredContent").is_none());
    }

    /// T4 — emitted structuredContent validates against the declared outputSchema
    /// for both the flat and the grouped value (using the same builders the
    /// real serializer would feed).
    #[test]
    fn t4_emitted_structured_content_validates_against_schema() {
        let schema = tool_output_schema("crw_search").expect("crw_search has outputSchema");
        let validator = jsonschema::validator_for(&schema).expect("schema compiles");

        for value in [representative_search_value(), grouped_search_value()] {
            let resp = tool_result_response(json!(1), "crw_search", Ok(value.clone()));
            let structured = result_of(&resp)["structuredContent"].clone();
            let errors: Vec<String> = validator
                .iter_errors(&structured)
                .map(|e| e.to_string())
                .collect();
            assert!(
                errors.is_empty(),
                "structuredContent failed schema validation for {value}:\n{}",
                errors.join("\n")
            );
        }
    }

    /// T5 — the helper is the single source of truth: present for crw_search,
    /// absent for crw_scrape, with the expected required-field structure.
    #[test]
    fn t5_tool_output_schema_helper() {
        let schema = tool_output_schema("crw_search").expect("crw_search has outputSchema");
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().expect("required array");
        assert_eq!(required, &vec![json!("success"), json!("data")]);
        assert_eq!(schema["properties"]["data"]["type"], "object");
        let data_required = schema["properties"]["data"]["required"]
            .as_array()
            .expect("data.required array");
        assert!(data_required.iter().any(|v| v == "results"));

        assert!(
            tool_output_schema("crw_scrape").is_none(),
            "crw_scrape declares no outputSchema"
        );
    }

    /// T6 — the additionalProperties:false guard is scoped to inputSchema only;
    /// the new outputSchema must not set it (the conditional SearchResult fields
    /// would make it falsely reject real responses).
    #[test]
    fn t6_output_schema_does_not_set_additional_properties_false() {
        let defs = tool_definitions(false);
        let search = tool_by_name(&defs, "crw_search");
        let ap = search["outputSchema"].get("additionalProperties");
        assert!(
            ap.is_none() || ap.and_then(|v| v.as_bool()) != Some(false),
            "crw_search outputSchema must not set additionalProperties:false"
        );
    }
}
