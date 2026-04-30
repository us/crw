use clap::{Parser, Subcommand};
use crw_core::config::AppConfig;
use crw_server::state::AppState;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "crw-server", about = "CRW web scraper API server")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Download LightPanda and create a local config for JS rendering
    Setup,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Setup) => {
            crw_server::setup::run_setup().await;
        }
        None => {
            run_server().await;
        }
    }
}

async fn run_server() {
    // Initialize tracing.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // Eagerly register Prometheus metrics so alert rules see present series at boot.
    crw_core::metrics::init();

    // Load configuration.
    let config = match AppConfig::load() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to load configuration: {e}");
            std::process::exit(1);
        }
    };

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
            "CRW_CDP_URL is set but is only honored by `crw` (CLI). \
             In server/MCP mode use [renderer.lightpanda.ws_url] / [renderer.chrome.ws_url] \
             or CRW_RENDERER__LIGHTPANDA__WS_URL / CRW_RENDERER__CHROME__WS_URL."
        );
    }

    if config.extraction.llm.is_some() {
        tracing::info!("LLM structured extraction: enabled");
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
