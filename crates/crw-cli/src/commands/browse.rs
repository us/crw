//! Browse subcommand — MCP server for interactive browser automation over CDP.

use clap::Args;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

#[derive(Args)]
pub struct BrowseArgs {
    /// CDP WebSocket URL (e.g. ws://localhost:9222).
    #[arg(long, env = "CRW_BROWSE_WS_URL", default_value = "ws://localhost:9222")]
    pub ws_url: String,

    /// Default per-page timeout in milliseconds.
    #[arg(long, env = "CRW_BROWSE_PAGE_TIMEOUT_MS", default_value_t = 30_000)]
    pub page_timeout_ms: u64,

    /// Optional Chrome/Chromium CDP endpoint used as a fallback for tools
    /// that Lightpanda implements as no-ops (`screenshot`). Without it,
    /// those tools return `NOT_IMPLEMENTED`.
    #[arg(long, env = "CRW_BROWSE_CHROME_WS_URL")]
    pub chrome_ws_url: Option<String>,
}

pub async fn run(args: BrowseArgs) -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let config = crw_browse::server::BrowseConfig {
        ws_url: args.ws_url.clone(),
        page_timeout: Duration::from_millis(args.page_timeout_ms),
        chrome_ws_url: args.chrome_ws_url,
    };

    tracing::info!(ws_url = %config.ws_url, "starting crw browse");

    use rmcp::ServiceExt;
    let service = crw_browse::server::CrwBrowse::new(config)
        .serve(rmcp::transport::stdio())
        .await
        .inspect_err(|e| tracing::error!("serve error: {e:?}"))?;
    service.waiting().await?;
    Ok(())
}
