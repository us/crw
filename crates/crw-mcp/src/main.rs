//! MCP (Model Context Protocol) server for the CRW web scraper.
//!
//! Supports two modes:
//!
//! - **Embedded (default)** — Self-contained scraping engine. No external server needed.
//! - **Proxy** — Forwards tool calls to a remote CRW server over HTTP.
//!
//! Mode selection: if `--api-url` or `CRW_API_URL` is set, proxy mode is used.
//! Otherwise, embedded mode is used (requires the `embedded` feature, on by default).
//!
//! # Tools
//!
//! - `crw_scrape` — scrape a single URL
//! - `crw_crawl` — start an async BFS crawl
//! - `crw_check_crawl_status` — poll crawl job status
//! - `crw_map` — discover URLs on a website
//! - `crw_search` — web search (proxy/cloud mode only)
//!
//! # Usage
//!
//! ```bash
//! # Embedded mode (default — no server needed)
//! crw-mcp
//!
//! # Proxy mode — connect to a remote server
//! crw-mcp --api-url https://fastcrw.com/api --api-key fc-xxx
//! ```

use clap::Parser;
use crw_core::mcp::{
    JsonRpcRequest, JsonRpcResponse, ProtocolResult, handle_protocol_method, tool_result_response,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[cfg(feature = "embedded")]
use crw_renderer::browser;

const SERVER_NAME: &str = "crw-mcp";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

// --- CLI ---

#[derive(Parser)]
#[command(name = "crw-mcp", about = "MCP server for the CRW web scraper")]
struct Cli {
    /// Remote CRW server URL. Enables proxy mode.
    /// Without this flag, runs in embedded mode (self-contained).
    #[arg(long, env = "CRW_API_URL")]
    api_url: Option<String>,

    /// API key for remote server authentication.
    #[arg(long, env = "CRW_API_KEY")]
    api_key: Option<String>,

    /// Config file path (embedded mode only, overrides config.local.toml).
    #[arg(long, env = "CRW_CONFIG")]
    config: Option<String>,
}

// --- Backend ---

enum Backend {
    Proxy {
        client: reqwest::Client,
        base_url: String,
        api_key: Option<String>,
    },
    #[cfg(feature = "embedded")]
    Embedded { state: crw_server::state::AppState },
}

impl Backend {
    async fn call_tool(&self, tool_name: &str, args: Value) -> Result<Value, String> {
        match self {
            Backend::Proxy {
                client,
                base_url,
                api_key,
            } => proxy_call_tool(client, base_url, api_key, tool_name, args).await,
            #[cfg(feature = "embedded")]
            Backend::Embedded { state } => {
                crw_server::routes::mcp::call_tool(state, tool_name, args).await
            }
        }
    }

    fn is_proxy(&self) -> bool {
        matches!(self, Backend::Proxy { .. })
    }

    async fn handle_request(&self, req: JsonRpcRequest) -> Option<JsonRpcResponse> {
        // Handle common protocol methods via shared logic.
        match handle_protocol_method(SERVER_NAME, SERVER_VERSION, &req, self.is_proxy()) {
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

                let result = self.call_tool(tool_name, arguments).await;
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
}

// --- Proxy mode HTTP dispatch ---

async fn proxy_call_tool(
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
        "crw_search" => {
            let resp = client
                .post(format!("{base_url}/v1/search"))
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
        let end = s.floor_char_boundary(max);
        &s[..end]
    }
}

// --- Main ---

#[tokio::main]
async fn main() {
    // Log to stderr so stdout stays clean for MCP protocol.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "crw_mcp=info".parse().unwrap()),
        )
        .init();

    let cli = Cli::parse();

    let backend = if let Some(api_url) = cli.api_url {
        tracing::info!("Starting {SERVER_NAME} v{SERVER_VERSION} (proxy mode)");
        tracing::info!("API URL: {api_url}");

        let client = reqwest::Client::builder()
            .redirect(crw_core::url_safety::safe_redirect_policy())
            .timeout(std::time::Duration::from_secs(120))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("reqwest client build failed");

        Backend::Proxy {
            client,
            base_url: api_url,
            api_key: cli.api_key,
        }
    } else {
        #[cfg(feature = "embedded")]
        {
            tracing::info!("Starting {SERVER_NAME} v{SERVER_VERSION} (embedded mode)");

            // Set CRW_CONFIG env var if --config was provided, so AppConfig::load() picks it up.
            if let Some(ref config_path) = cli.config {
                // SAFETY: This runs before any other threads read CRW_CONFIG.
                // AppConfig::load() is called immediately after on the same thread.
                unsafe { std::env::set_var("CRW_CONFIG", config_path) };
            }

            let mut config = crw_core::config::AppConfig::load().unwrap_or_else(|e| {
                tracing::warn!("Failed to load config, using defaults: {e}");
                crw_core::config::AppConfig {
                    server: Default::default(),
                    renderer: Default::default(),
                    crawler: Default::default(),
                    extraction: Default::default(),
                    auth: Default::default(),
                }
            });

            // Auto-spawn a headless browser for JS rendering.
            // Priority: LightPanda (native/Docker) → Chrome/Chromium.
            // Skip only if the user explicitly set a renderer via env var.
            // Config file renderers (e.g. config.default.toml) may reference
            // Docker services that aren't running locally, so we don't trust them.
            let user_configured_renderer = std::env::var("CRW_RENDERER__LIGHTPANDA__WS_URL")
                .is_ok()
                || std::env::var("CRW_RENDERER__CHROME__WS_URL").is_ok()
                || std::env::var("CRW_RENDERER__PLAYWRIGHT__WS_URL").is_ok();

            let _browser_guards = if !user_configured_renderer {
                let browsers = browser::spawn_all_headless().await;
                if browsers.is_empty() {
                    tracing::info!(
                        "No browser found — JS rendering disabled. \
                         Install LightPanda or Chrome for full SPA support."
                    );
                }
                let mut guards = Vec::new();
                for (guard, ws_url, kind) in browsers {
                    match kind {
                        browser::RendererKind::LightPanda => {
                            config.renderer.lightpanda =
                                Some(crw_core::config::CdpEndpoint { ws_url });
                        }
                        browser::RendererKind::Chrome => {
                            config.renderer.chrome = Some(crw_core::config::CdpEndpoint { ws_url });
                        }
                    }
                    guards.push(guard);
                }
                guards
            } else {
                tracing::info!("CDP renderer already configured — skipping auto-spawn");
                Vec::new()
            };

            let state = crw_server::state::AppState::new(config);

            // Run the MCP loop. _browser_guards keeps browsers alive until shutdown.
            let backend = Backend::Embedded { state };
            run_stdio_loop(backend).await;

            // Drop browser guards explicitly (kills browser processes).
            drop(_browser_guards);
            return;
        }

        #[cfg(not(feature = "embedded"))]
        {
            tracing::error!(
                "Embedded mode not available (compiled without 'embedded' feature). \
                 Use --api-url to connect to a remote CRW server."
            );
            std::process::exit(1);
        }
    };

    run_stdio_loop(backend).await;
}

async fn run_stdio_loop(backend: Backend) {
    let mut stdout = tokio::io::stdout();
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

        if let Some(resp) = backend.handle_request(req).await {
            let out = serde_json::to_string(&resp).unwrap();
            tracing::debug!("→ {out}");
            let _ = stdout.write_all(out.as_bytes()).await;
            let _ = stdout.write_all(b"\n").await;
            let _ = stdout.flush().await;
        }
    }
}
