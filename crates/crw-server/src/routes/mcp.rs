use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use crw_core::mcp::{
    JsonRpcRequest, JsonRpcResponse, ProtocolResult, handle_protocol_method, tool_result_response,
};
use crw_core::types::{CrawlRequest, MapRequest, ScrapeRequest, SearchRequest};
use crw_crawl::crawl::{DiscoverOptions, discover_urls};
use crw_crawl::single::scrape_url;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::state::{AppState, validate_crawl_renderer};

const SERVER_NAME: &str = "crw";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Validate URL safety for MCP tool calls (same checks as REST API routes).
pub async fn validate_url(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("Invalid URL: {e}"))?;
    crw_core::url_safety::validate_safe_url_resolved(&parsed).await
}

pub async fn call_tool(state: &AppState, tool_name: &str, args: Value) -> Result<Value, String> {
    match tool_name {
        "crw_scrape" => {
            let req: ScrapeRequest =
                serde_json::from_value(args).map_err(|e| format!("invalid arguments: {e}"))?;
            validate_url(&req.url).await?;
            let llm_config = state.config.extraction.llm.as_ref();
            let user_agent = &state.config.crawler.user_agent;
            let default_stealth =
                state.config.crawler.stealth.enabled && state.config.crawler.stealth.inject_headers;
            let deadline = crw_core::Deadline::from_request_ms(
                state
                    .config
                    .effective_deadline_ms(req.deadline_ms, req.wait_for),
            );
            let data = scrape_url(
                &req,
                &state.renderer,
                llm_config,
                &state.config.extraction,
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
            validate_url(&req.url).await?;
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
            validate_url(&req.url).await?;
            let max_depth = req
                .max_depth
                .unwrap_or(state.config.crawler.default_max_depth);
            let result = discover_urls(DiscoverOptions {
                base_url: &req.url,
                max_depth,
                use_sitemap: req.use_sitemap,
                renderer: &state.renderer,
                max_concurrency: state.config.crawler.max_concurrency,
                requests_per_second: state.config.crawler.requests_per_second,
                user_agent: &state.config.crawler.user_agent,
                proxy: state.config.crawler.proxy.clone(),
                deadline_ms_per_page: state.config.effective_deadline_ms(None, None),
                per_host_max_concurrent: state.config.crawler.per_host_max_concurrent,
                crawl_fallback: req.crawl_fallback,
                url_filter: state.url_filter.clone(),
            })
            .await
            .map_err(|e| format!("{e}"))?;
            Ok(json!({
                "success": true,
                "links": result.urls,
                "droppedActionCount": result.dropped_action_count,
                "strippedTrackingCount": result.stripped_tracking_count,
            }))
        }
        "crw_search" => {
            let req: SearchRequest =
                serde_json::from_value(args).map_err(|e| format!("invalid arguments: {e}"))?;
            let resp = crate::routes::search::search_inner(state, req)
                .await
                .map_err(|e| format!("{e}"))?;
            serde_json::to_value(&resp).map_err(|e| format!("serialize error: {e}"))
        }
        "crw_parse_file" => {
            use base64::Engine;
            let b64 = args
                .get("contentBase64")
                .and_then(|v| v.as_str())
                .ok_or("missing required parameter: contentBase64")?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64.trim())
                .map_err(|e| format!("invalid base64 in contentBase64: {e}"))?;

            // Optional ScrapeRequest-shaped fields (formats/jsonSchema/parsers).
            let req: ScrapeRequest = serde_json::from_value(args.clone()).unwrap_or_default();
            let llm_config = state.config.extraction.llm.as_ref();
            let filename = args
                .get("filename")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let source = crw_crawl::pdf::PdfSource {
                source_url: format!("upload://{}", filename.as_deref().unwrap_or("document.pdf")),
                status_code: 200,
                elapsed_ms: 0,
                source_filename: filename,
            };
            let mut data = crw_crawl::pdf::convert_pdf_bytes_strict(bytes, &req, source)
                .await
                .map_err(|(crw_err, _)| format!("{crw_err}"))?;
            crw_crawl::pdf::apply_llm_formats(&mut data, &req, llm_config)
                .await
                .map_err(|e| format!("{e}"))?;
            serde_json::to_value(&data).map_err(|e| format!("serialize error: {e}"))
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
            Some(tool_result_response(id, tool_name, result))
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

    // MCP 2025-06-18 requires clients to send `MCP-Protocol-Version` on every
    // post-initialize request. We TOLERATE it: read for observability, never
    // reject on presence, absence, or mismatch. Hard validation is deferred
    // until client adoption is confirmed. Do NOT add a reject branch here
    // without updating the header-tolerance test in tests/mcp.rs.
    let _client_protocol = headers
        .get("mcp-protocol-version")
        .and_then(|v| v.to_str().ok());

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
