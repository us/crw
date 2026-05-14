//! CRW — Unified CLI for web scraping, crawling, and search.
//!
//! # Usage
//!
//! ```bash
//! # Default: scrape (backwards compatible)
//! crw example.com
//! crw example.com --format json
//!
//! # Explicit subcommands
//! crw scrape example.com
//! crw search "rust web scraper"
//! crw crawl example.com --depth 3
//! crw map example.com
//! crw serve --port 3000
//! crw mcp
//! crw browse
//! crw setup
//! ```

mod commands;

use clap::{Parser, Subcommand};
use commands::scrape::Format;

#[derive(Parser)]
#[command(
    name = "crw",
    about = "Web scraper for AI agents",
    long_about = "Unified CLI for web scraping, crawling, search, and serving.\n\n\
        The fastest web scraper built for AI agents and LLM data pipelines.\n\n\
        Examples:\n  \
        crw example.com                    # Scrape URL (default mode)\n  \
        crw scrape example.com --format json\n  \
        crw search \"rust web scraper\"     # Web search via SearXNG\n  \
        crw crawl example.com --depth 3   # BFS crawl\n  \
        crw map example.com               # Discover URLs\n  \
        crw serve --port 3000             # Start REST API server\n  \
        crw mcp                           # Start MCP server\n  \
        crw browse                        # Start browser automation MCP\n  \
        crw setup                         # Interactive setup wizard"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    // --- Default scrape mode (backwards compat) ---
    /// URL to scrape (when no subcommand is given)
    #[arg(value_name = "URL", conflicts_with = "command")]
    url: Option<String>,

    /// Output format (for default scrape mode)
    #[arg(short, long, value_enum, default_value = "markdown")]
    format: Option<Format>,

    /// Write output to file instead of stdout
    #[arg(short, long, value_name = "FILE")]
    output: Option<String>,

    /// Disable main content extraction (return full page content)
    #[arg(long)]
    raw: bool,

    /// Enable JavaScript rendering
    #[arg(long)]
    js: bool,

    /// Extract only elements matching this CSS selector
    #[arg(long, value_name = "SELECTOR")]
    css: Option<String>,

    /// Extract only elements matching this XPath expression
    #[arg(long, value_name = "EXPR")]
    xpath: Option<String>,

    /// HTTP, HTTPS, or SOCKS5 proxy URL
    #[arg(long, value_name = "URL")]
    proxy: Option<String>,

    /// Enable stealth mode
    #[arg(long)]
    stealth: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Scrape a single URL and output content
    Scrape(commands::scrape::ScrapeArgs),

    /// Web search via SearXNG
    Search(commands::search::SearchArgs),

    /// BFS crawl a website starting from a URL
    Crawl(commands::crawl::CrawlArgs),

    /// Discover URLs on a website via sitemap and crawling
    Map(commands::map::MapArgs),

    /// Start the REST API server (Firecrawl-compatible)
    Serve(commands::serve::ServeArgs),

    /// Start the MCP (Model Context Protocol) server
    Mcp(commands::mcp::McpArgs),

    /// Start the browser automation MCP server
    Browse(commands::browse::BrowseArgs),

    /// Interactive setup wizard (Cloud or Local)
    Setup(commands::setup::SetupArgs),
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Scrape(args)) => {
            commands::scrape::run(args).await;
        }
        Some(Commands::Search(args)) => {
            commands::search::run(args).await;
        }
        Some(Commands::Crawl(args)) => {
            commands::crawl::run(args).await;
        }
        Some(Commands::Map(args)) => {
            commands::map::run(args).await;
        }
        Some(Commands::Serve(args)) => {
            commands::serve::run(args).await;
        }
        Some(Commands::Mcp(args)) => {
            commands::mcp::run(args).await;
        }
        Some(Commands::Browse(args)) => {
            if let Err(e) = commands::browse::run(args).await {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
        Some(Commands::Setup(args)) => {
            commands::setup::run(args).await;
        }
        None => {
            // Default mode: scrape (backwards compatible)
            if let Some(url) = cli.url {
                let args = commands::scrape::ScrapeArgs {
                    url,
                    format: cli.format.unwrap_or(Format::Markdown),
                    output: cli.output,
                    raw: cli.raw,
                    js: cli.js,
                    css: cli.css,
                    xpath: cli.xpath,
                    proxy: cli.proxy,
                    stealth: cli.stealth,
                };
                commands::scrape::run(args).await;
            } else {
                // No URL provided — show help
                use clap::CommandFactory;
                Cli::command().print_help().unwrap();
                println!();
            }
        }
    }
}
