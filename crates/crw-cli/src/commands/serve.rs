//! Serve subcommand — start the REST API server.
//!
//! Implements the Firecrawl-compatible API at `/v1/*` endpoints.

use clap::Args;
use crw_core::config::AppConfig;
use crw_server::state::AppState;
use tracing_subscriber::EnvFilter;

#[derive(Args)]
pub struct ServeArgs {
    /// Host to bind to
    #[arg(long, env = "CRW_HOST", default_value = "0.0.0.0")]
    pub host: String,

    /// Port to bind to
    #[arg(short, long, env = "CRW_PORT", default_value = "3000")]
    pub port: u16,

    /// Config file path (overrides default config search)
    #[arg(long, env = "CRW_CONFIG")]
    pub config: Option<String>,
}

pub async fn run(args: ServeArgs) {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // Eagerly register Prometheus metrics
    crw_core::metrics::init();

    // Set CRW_CONFIG env var if --config was provided
    if let Some(ref config_path) = args.config {
        // SAFETY: This runs before config is loaded on the same thread
        unsafe { std::env::set_var("CRW_CONFIG", config_path) };
    }

    // Load configuration
    let mut config = match AppConfig::load() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to load configuration: {e}");
            std::process::exit(1);
        }
    };

    // Override host/port from CLI args
    config.server.host = args.host.clone();
    config.server.port = args.port;

    let addr = format!("{}:{}", config.server.host, config.server.port);
    tracing::info!("Starting CRW on {addr}");
    tracing::info!("Renderer mode: {:?}", config.renderer.mode);
    tracing::info!(
        "Renderer render_js_default: {:?}",
        config.renderer.render_js_default
    );
    if let Some(lp) = &config.renderer.lightpanda {
        tracing::info!("Lightpanda CDP: {}", lp.ws_url);
    }
    if let Some(ch) = &config.renderer.chrome {
        tracing::info!("Chrome CDP: {}", ch.ws_url);
    }
    if std::env::var("CRW_CDP_URL").is_ok() {
        tracing::warn!(
            "CRW_CDP_URL is set but is only honored by `crw` (CLI scrape mode). \
             In server mode use [renderer.lightpanda.ws_url] / [renderer.chrome.ws_url] \
             or CRW_RENDERER__LIGHTPANDA__WS_URL / CRW_RENDERER__CHROME__WS_URL."
        );
    }

    if config.extraction.llm.is_some() {
        tracing::info!("LLM structured extraction: enabled");
    }

    // Boot guard: when SaaS fronts opencore it sets `CRW_DISABLE_SERVER_LLM_KEY=1`
    let disable_server_key = std::env::var("CRW_DISABLE_SERVER_LLM_KEY")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if disable_server_key
        && config
            .extraction
            .llm
            .as_ref()
            .is_some_and(|c| !c.api_key.is_empty())
    {
        tracing::error!(
            "CRW_DISABLE_SERVER_LLM_KEY=1 but [extraction.llm].api_key is also configured. \
             This is forbidden in SaaS-fronted deploys (refusing to boot)."
        );
        std::process::exit(1);
    }

    let state = match AppState::new(config) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to build application state: {e}");
            std::process::exit(1);
        }
    };
    tracing::info!(
        "JS renderers in fallback order: {:?}",
        state.renderer.js_renderer_names()
    );
    if state.renderer.js_renderer_names().is_empty() {
        tracing::warn!("No CDP renderer active — JS rendering disabled");
    }

    // Issue #35 transparency: log effective deadline values
    let baseline_default_ms = state.config.request.deadline_ms_default;
    let ladder_min_ms = state.config.renderer.min_deadline_for_full_ladder_ms();
    let effective_default_ms = state.config.effective_deadline_ms(None, None);
    let baseline_outer_secs = state.config.server.request_timeout_secs;
    let effective_outer_secs = state.config.effective_request_timeout_secs();
    if state.config.request.auto_extend_deadline_for_ladder
        && effective_default_ms > baseline_default_ms
    {
        tracing::info!(
            deadline_ms_default = baseline_default_ms,
            ladder_min_ms,
            effective_default_ms,
            outer_timeout_secs_baseline = baseline_outer_secs,
            outer_timeout_secs_effective = effective_outer_secs,
            "request.auto_extend_deadline_for_ladder is on; default request \
             deadline auto-raised so the configured renderer ladder \
             (http+lightpanda+chrome+overhead) can run uncrushed."
        );
    }

    // Capture handles before `state` is consumed by `create_app`
    let renderer = std::sync::Arc::clone(&state.renderer);
    let pool_drain =
        std::time::Duration::from_secs(state.config.renderer.chrome_pool.shutdown_drain_secs);

    let app = crw_server::app::create_app(state);

    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Failed to bind to {addr}: {e}");
            std::process::exit(1);
        }
    };

    tracing::info!("CRW ready at http://{addr}");

    let server = axum::serve(listener, app).with_graceful_shutdown(shutdown_signal());

    if let Err(e) = server.await {
        tracing::error!("Server error: {e}");
        std::process::exit(1);
    }

    // HTTP layer is quiesced; now drain the chrome pool
    renderer.shutdown_chrome_pool(pool_drain).await;

    tracing::info!("Server shut down gracefully");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("Received Ctrl+C, shutting down..."),
        _ = terminate => tracing::info!("Received SIGTERM, shutting down..."),
    }
}
