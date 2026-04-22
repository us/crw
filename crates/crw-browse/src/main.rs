use anyhow::Result;
use clap::Parser;
use crw_browse::server::{BrowseConfig, CrwBrowse};
use rmcp::{ServiceExt, transport::stdio};
use std::time::Duration;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "crw-browse",
    about = "MCP server for interactive browser automation"
)]
struct Cli {
    /// CDP WebSocket URL (e.g. ws://localhost:9222).
    #[arg(long, env = "CRW_BROWSE_WS_URL", default_value = "ws://localhost:9222")]
    ws_url: String,

    /// Default per-page timeout in milliseconds.
    #[arg(long, env = "CRW_BROWSE_PAGE_TIMEOUT_MS", default_value_t = 30_000)]
    page_timeout_ms: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let cli = Cli::parse();
    let config = BrowseConfig {
        ws_url: cli.ws_url,
        page_timeout: Duration::from_millis(cli.page_timeout_ms),
    };

    tracing::info!(ws_url = %config.ws_url, "starting crw-browse");

    let service = CrwBrowse::new(config)
        .serve(stdio())
        .await
        .inspect_err(|e| tracing::error!("serve error: {e:?}"))?;
    service.waiting().await?;
    Ok(())
}
