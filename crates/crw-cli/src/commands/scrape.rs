//! Scrape subcommand — fetch a single URL and extract content.

use clap::{Args, ValueEnum};
use crw_core::config::{RendererConfig, RendererMode, StealthConfig};
use crw_core::types::{OutputFormat, ScrapeRequest};
use crw_crawl::single::scrape_url;
use crw_renderer::FallbackRenderer;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, ValueEnum)]
pub enum Format {
    Markdown,
    Json,
    Html,
    Rawhtml,
    Text,
    Links,
}

#[derive(Args)]
pub struct ScrapeArgs {
    /// URL to scrape (http or https)
    pub url: String,

    /// Output format
    #[arg(short, long, value_enum, default_value = "markdown")]
    pub format: Format,

    /// Write output to file instead of stdout
    #[arg(short, long, value_name = "FILE")]
    pub output: Option<String>,

    /// Disable main content extraction (return full page content)
    #[arg(long)]
    pub raw: bool,

    /// Enable JavaScript rendering (auto-detects LightPanda/Chrome, or use CRW_CDP_URL)
    #[arg(long)]
    pub js: bool,

    /// Extract only elements matching this CSS selector
    #[arg(long, value_name = "SELECTOR")]
    pub css: Option<String>,

    /// Extract only elements matching this XPath expression
    #[arg(long, value_name = "EXPR")]
    pub xpath: Option<String>,

    /// HTTP, HTTPS, or SOCKS5 proxy URL (e.g. http://user:pass@host:port or socks5://user:pass@host:1080)
    #[arg(long, value_name = "URL")]
    pub proxy: Option<String>,

    /// Enable stealth mode (rotate user agents, inject browser headers)
    #[arg(long)]
    pub stealth: bool,
}

pub async fn run(mut args: ScrapeArgs) {
    // Auto-prepend https:// if no scheme is provided
    if !args.url.contains("://") {
        args.url = format!("https://{}", args.url);
    }

    // Build renderer config — auto-detect browser if --js
    let mut renderer_config = RendererConfig::default();
    let _browser_guards = if args.js {
        // Check if user explicitly set a CDP URL (backwards compat)
        if let Ok(ws_url) = std::env::var("CRW_CDP_URL") {
            renderer_config.lightpanda = Some(crw_core::config::CdpEndpoint { ws_url });
            Vec::new()
        } else {
            // Auto-detect: spawn all available browsers for fallback chain
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
        // HTTP-only — no CDP
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

    // For JSON output, we serialize the full ScrapeData — not LLM structured extraction.
    // Request all formats we might need for the output.
    let request_formats = match args.format {
        Format::Markdown => vec![OutputFormat::Markdown],
        Format::Json => vec![
            OutputFormat::Markdown,
            OutputFormat::Html,
            OutputFormat::Links,
        ],
        Format::Html => vec![OutputFormat::Html],
        Format::Rawhtml => vec![OutputFormat::RawHtml],
        Format::Text => vec![OutputFormat::PlainText],
        Format::Links => vec![OutputFormat::Links],
    };

    let req = ScrapeRequest {
        url: args.url,
        formats: request_formats,
        only_main_content: !args.raw,
        render_js: if args.js { Some(true) } else { None },
        wait_for: None,
        include_tags: vec![],
        exclude_tags: vec![],
        json_schema: None,
        headers: HashMap::new(),
        css_selector: args.css,
        xpath: args.xpath,
        chunk_strategy: None,
        query: None,
        filter_mode: None,
        top_k: None,
        proxy: args.proxy,
        stealth: if args.stealth { Some(true) } else { None },
        actions: None,
        extract: None,
        llm_api_key: None,
        llm_provider: None,
        llm_model: None,
        base_url: None,
        summary_prompt: None,
        max_content_chars: None,
        renderer: None,
        deadline_ms: None,
        debug: None,
    };

    let cli_deadline = crw_core::Deadline::from_request_ms(req.deadline_ms.unwrap_or(8000));
    let cli_extraction_cfg = crw_core::config::ExtractionConfig::default();
    let data = match scrape_url(
        &req,
        &renderer,
        None,
        &cli_extraction_cfg,
        "crw/0.7.0",
        args.stealth,
        None,
        cli_deadline,
    )
    .await
    {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    let content = match args.format {
        Format::Markdown => data.markdown.unwrap_or_default(),
        Format::Json => match serde_json::to_string_pretty(&data) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: failed to serialize JSON: {e}");
                std::process::exit(1);
            }
        },
        Format::Html => data.html.unwrap_or_default(),
        Format::Rawhtml => data.raw_html.unwrap_or_default(),
        Format::Text => data.plain_text.unwrap_or_default(),
        Format::Links => data.links.unwrap_or_default().join("\n"),
    };

    match args.output {
        Some(path) => {
            if let Err(e) = std::fs::write(&path, &content) {
                eprintln!("error: failed to write to {path}: {e}");
                std::process::exit(1);
            }
        }
        None => print!("{content}"),
    }
}
