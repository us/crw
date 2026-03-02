mod app;
mod error;
mod middleware;
mod routes;
mod state;

use crw_core::config::AppConfig;
use state::AppState;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    // Initialize tracing.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

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
    tracing::info!("Renderer mode: {}", config.renderer.mode);

    if config.extraction.llm.is_some() {
        tracing::info!("LLM structured extraction: enabled");
    }

    let state = AppState::new(config);
    let app = app::create_app(state);

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
