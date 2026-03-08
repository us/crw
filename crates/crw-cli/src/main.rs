use clap::{Parser, ValueEnum};
use crw_core::config::{RendererConfig, StealthConfig};
use crw_core::types::{OutputFormat, ScrapeRequest};
use crw_crawl::single::scrape_url;
use crw_renderer::FallbackRenderer;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Parser)]
#[command(
    name = "crw",
    about = "Scrape a URL and output markdown, JSON, HTML, or plain text",
    long_about = "Lightweight web scraper. Fetches a URL and outputs clean content to stdout.\n\nExamples:\n  crw https://example.com\n  crw https://example.com --format json\n  crw https://example.com --raw -o page.md\n  crw https://example.com --css 'article' --format html"
)]
struct Cli {
    /// URL to scrape (http or https)
    url: String,

    /// Output format
    #[arg(short, long, value_enum, default_value = "markdown")]
    format: Format,

    /// Write output to file instead of stdout
    #[arg(short, long, value_name = "FILE")]
    output: Option<String>,

    /// Disable main content extraction (return full page content)
    #[arg(long)]
    raw: bool,

    /// Force JavaScript rendering via CDP (requires CRW_CDP_URL env var)
    #[arg(long)]
    js: bool,

    /// Extract only elements matching this CSS selector
    #[arg(long, value_name = "SELECTOR")]
    css: Option<String>,

    /// Extract only elements matching this XPath expression
    #[arg(long, value_name = "EXPR")]
    xpath: Option<String>,

    /// HTTP/HTTPS proxy URL (e.g. http://user:pass@host:port)
    #[arg(long, value_name = "URL")]
    proxy: Option<String>,

    /// Enable stealth mode (rotate user agents, inject browser headers)
    #[arg(long)]
    stealth: bool,
}

#[derive(Clone, ValueEnum)]
enum Format {
    Markdown,
    Json,
    Html,
    Rawhtml,
    Text,
    Links,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Build renderer config — enable CDP if --js and CRW_CDP_URL is set
    let mut renderer_config = RendererConfig::default();
    if cli.js {
        if let Ok(ws_url) = std::env::var("CRW_CDP_URL") {
            renderer_config.lightpanda = Some(crw_core::config::CdpEndpoint { ws_url });
        } else {
            eprintln!("warning: --js requires CRW_CDP_URL env var (e.g. ws://localhost:9222). Falling back to HTTP.");
        }
    } else {
        // HTTP-only — no CDP
        renderer_config.mode = "none".into();
    }

    let stealth_config = StealthConfig {
        enabled: cli.stealth,
        inject_headers: cli.stealth,
        ..Default::default()
    };

    let renderer = Arc::new(FallbackRenderer::new(
        &renderer_config,
        "crw/0.0.2",
        cli.proxy.as_deref(),
        &stealth_config,
    ));

    let output_format = match cli.format {
        Format::Markdown => OutputFormat::Markdown,
        Format::Json => OutputFormat::Json,
        Format::Html => OutputFormat::Html,
        Format::Rawhtml => OutputFormat::RawHtml,
        Format::Text => OutputFormat::PlainText,
        Format::Links => OutputFormat::Links,
    };

    let req = ScrapeRequest {
        url: cli.url,
        formats: vec![output_format],
        only_main_content: !cli.raw,
        render_js: if cli.js { Some(true) } else { None },
        wait_for: None,
        include_tags: vec![],
        exclude_tags: vec![],
        json_schema: None,
        headers: HashMap::new(),
        css_selector: cli.css,
        xpath: cli.xpath,
        chunk_strategy: None,
        query: None,
        filter_mode: None,
        top_k: None,
        proxy: cli.proxy,
        stealth: if cli.stealth { Some(true) } else { None },
    };

    let data = match scrape_url(&req, &renderer, None, "crw/0.0.2", cli.stealth).await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    let content = match cli.format {
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
        Format::Links => data
            .links
            .unwrap_or_default()
            .join("\n"),
    };

    match cli.output {
        Some(path) => {
            if let Err(e) = std::fs::write(&path, &content) {
                eprintln!("error: failed to write to {path}: {e}");
                std::process::exit(1);
            }
        }
        None => print!("{content}"),
    }
}
