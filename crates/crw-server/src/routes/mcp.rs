use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use crw_core::mcp::{
    JsonRpcRequest, JsonRpcResponse, ProtocolResult, handle_protocol_method, tool_result_response,
};
use crw_core::types::{CrawlRequest, MapRequest, ScrapeRequest};
use crw_crawl::crawl::{DiscoverOptions, discover_urls};
use crw_crawl::single::scrape_url;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::state::{AppState, validate_crawl_renderer};

const SERVER_NAME: &str = "crw";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Validate URL safety for MCP tool calls (same checks as REST API routes).
pub fn validate_url(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("Invalid URL: {e}"))?;
    crw_core::url_safety::validate_safe_url(&parsed)
}

pub async fn call_tool(state: &AppState, tool_name: &str, args: Value) -> Result<Value, String> {
    match tool_name {
        "crw_scrape" => {
            let req: ScrapeRequest =
                serde_json::from_value(args).map_err(|e| format!("invalid arguments: {e}"))?;
            validate_url(&req.url)?;
            let llm_config = state.config.extraction.llm.as_ref();
            let user_agent = &state.config.crawler.user_agent;
            let default_stealth =
                state.config.crawler.stealth.enabled && state.config.crawler.stealth.inject_headers;
            let deadline = crw_core::Deadline::from_request_ms(
                req.deadline_ms
                    .unwrap_or(state.config.request.deadline_ms_default),
            );
            let data = scrape_url(
                &req,
                &state.renderer,
                llm_config,
                user_agent,
                default_stealth,
                state.config.renderer.render_js_default,
                deadline,
            )
            .await
            .map_err(|e| format!("{e}"))?;
            serde_json::to_value(&data).map_err(|e| format!("serialize error: {e}"))
        }
        "crw_crawl" => {
            let req: CrawlRequest =
                serde_json::from_value(args).map_err(|e| format!("invalid arguments: {e}"))?;
            validate_url(&req.url)?;
            validate_crawl_renderer(&req, state).map_err(|e| format!("{e}"))?;
            let id = state.start_crawl_job(req).await;
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
            let urls = discover_urls(DiscoverOptions {
                base_url: &req.url,
                max_depth,
                use_sitemap: req.use_sitemap,
                renderer: &state.renderer,
                max_concurrency: state.config.crawler.max_concurrency,
                requests_per_second: state.config.crawler.requests_per_second,
                user_agent: &state.config.crawler.user_agent,
                proxy: state.config.crawler.proxy.clone(),
                deadline_ms_per_page: state.config.request.deadline_ms_default,
            })
            .await
            .map_err(|e| format!("{e}"))?;
            Ok(json!({"success": true, "links": urls}))
        }
        _ => Err(format!("unknown tool: {tool_name}")),
    }
}

pub async fn handle_request(state: &AppState, req: JsonRpcRequest) -> Option<JsonRpcResponse> {
    // Handle common protocol methods via shared logic.
    match handle_protocol_method(SERVER_NAME, SERVER_VERSION, &req, false) {
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

            let result = call_tool(state, tool_name, arguments).await;
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
