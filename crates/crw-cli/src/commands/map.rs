//! Map subcommand — discover URLs on a website via sitemap and crawling.

use clap::{Args, ValueEnum};
use crw_core::config::{RendererConfig, RendererMode, StealthConfig};
use crw_crawl::crawl::{DiscoverOptions, discover_urls};
use crw_renderer::FallbackRenderer;
use std::sync::Arc;

#[derive(Clone, ValueEnum)]
pub enum MapFormat {
    /// One URL per line
    Text,
    /// JSON array
    Json,
}

#[derive(Args)]
pub struct MapArgs {
    /// Base URL to discover links from
    pub url: String,

    /// Maximum depth for link discovery
    #[arg(short, long, default_value = "2")]
    pub depth: u32,

    /// Output format
    #[arg(short, long, value_enum, default_value = "text")]
    pub format: MapFormat,

    /// Skip sitemap discovery, only use HTML crawling
    #[arg(long)]
    pub no_sitemap: bool,

    /// Skip HTML crawl fallback, only use sitemap
    #[arg(long)]
    pub sitemap_only: bool,

    /// Enable JavaScript rendering
    #[arg(long)]
    pub js: bool,

    /// HTTP, HTTPS, or SOCKS5 proxy URL
    #[arg(long, value_name = "URL")]
    pub proxy: Option<String>,

    /// Enable stealth mode
    #[arg(long)]
    pub stealth: bool,

    /// Requests per second rate limit
    #[arg(long, default_value = "5.0")]
    pub rate_limit: f64,

    /// Maximum concurrent requests
    #[arg(long, default_value = "10")]
    pub concurrency: usize,

    /// Per-page timeout in milliseconds
    #[arg(long, default_value = "15000")]
    pub timeout: u64,
}

pub async fn run(mut args: MapArgs) {
    // Auto-prepend https:// if no scheme is provided
    if !args.url.contains("://") {
        args.url = format!("https://{}", args.url);
    }

    // Build renderer config
    let mut renderer_config = RendererConfig::default();
    let _browser_guards = if args.js {
        if let Ok(ws_url) = std::env::var("CRW_CDP_URL") {
            renderer_config.lightpanda = Some(crw_core::config::CdpEndpoint { ws_url });
            Vec::new()
        } else {
            let browsers = crw_renderer::browser::spawn_all_headless().await;
            if browsers.is_empty() {
                eprintln!(
                    "warning: --js requested but no browser found. \
                     Install LightPanda or Chrome for JS rendering. \
                     Falling back to HTTP."
                );
            }
            let mut guards = Vec::new();
            for (guard, ws_url, kind) in browsers {
                match kind {
                    crw_renderer::browser::RendererKind::LightPanda => {
                        renderer_config.lightpanda = Some(crw_core::config::CdpEndpoint { ws_url });
                    }
                    crw_renderer::browser::RendererKind::Chrome => {
                        renderer_config.chrome = Some(crw_core::config::CdpEndpoint { ws_url });
                    }
                }
                guards.push(guard);
            }
            guards
        }
    } else {
        renderer_config.mode = RendererMode::None;
        Vec::new()
    };

    let stealth_config = StealthConfig {
        enabled: args.stealth,
        inject_headers: args.stealth,
        ..Default::default()
    };

    let renderer = match FallbackRenderer::new(
        &renderer_config,
        "crw/0.7.0",
        args.proxy.as_deref(),
        &stealth_config,
    ) {
        Ok(r) => Arc::new(r),
        Err(e) => {
            eprintln!("error: failed to build renderer: {e}");
            std::process::exit(1);
        }
    };

    let opts = DiscoverOptions {
        base_url: &args.url,
        max_depth: args.depth,
        use_sitemap: !args.no_sitemap,
        crawl_fallback: !args.sitemap_only,
        renderer: &renderer,
        max_concurrency: args.concurrency,
        requests_per_second: args.rate_limit,
        user_agent: "crw/0.7.0",
        proxy: args.proxy,
        deadline_ms_per_page: args.timeout,
        per_host_max_concurrent: 2,
        url_filter: None,
    };

    let result = match discover_urls(opts).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: map failed: {e}");
            std::process::exit(1);
        }
    };

    match args.format {
        MapFormat::Text => {
            for url in &result.urls {
                println!("{url}");
            }
            eprintln!();
            eprintln!("Discovered {} URLs", result.urls.len());
            if result.dropped_action_count > 0 {
                eprintln!("  Dropped {} action URLs", result.dropped_action_count);
            }
            if result.stripped_tracking_count > 0 {
                eprintln!(
                    "  Stripped tracking from {} URLs",
                    result.stripped_tracking_count
                );
            }
        }
        MapFormat::Json => {
            let output = serde_json::json!({
                "success": true,
                "links": result.urls,
                "droppedActionCount": result.dropped_action_count,
                "strippedTrackingCount": result.stripped_tracking_count,
            });
            match serde_json::to_string_pretty(&output) {
                Ok(s) => println!("{s}"),
                Err(e) => {
                    eprintln!("error: failed to serialize JSON: {e}");
                    std::process::exit(1);
                }
            }
        }
    }
}
