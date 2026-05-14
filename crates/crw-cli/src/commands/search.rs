//! Search subcommand — web search via SearXNG.
//!
//! This is NEW functionality that was previously only available via the REST API.

use clap::{Args, ValueEnum};
use crw_search::{SearxngClient, SearxngParams, transform_flat};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, ValueEnum)]
pub enum SearchFormat {
    /// JSON output with full result details
    Json,
    /// Concise text output (title + URL per line)
    Text,
    /// Markdown output with links
    Markdown,
}

#[derive(Args)]
pub struct SearchArgs {
    /// Search query
    pub query: String,

    /// Maximum number of results to return
    #[arg(short, long, default_value = "10")]
    pub limit: u32,

    /// SearXNG instance URL
    #[arg(long, env = "CRW_SEARXNG_URL", default_value = "http://localhost:8080")]
    pub searxng_url: String,

    /// Output format
    #[arg(short, long, value_enum, default_value = "text")]
    pub format: SearchFormat,

    /// Search category (general, images, news, videos, etc.)
    #[arg(long)]
    pub category: Option<String>,

    /// Language code (e.g., en, de, fr)
    #[arg(long)]
    pub language: Option<String>,

    /// Time range filter (day, week, month, year)
    #[arg(long)]
    pub time_range: Option<String>,

    /// Safe search level (0 = off, 1 = moderate, 2 = strict)
    #[arg(long)]
    pub safesearch: Option<u8>,

    /// Request timeout in seconds
    #[arg(long, default_value = "30")]
    pub timeout: u64,
}

pub async fn run(args: SearchArgs) {
    let http = Arc::new(
        reqwest::Client::builder()
            .redirect(crw_core::url_safety::safe_redirect_policy())
            .build()
            .expect("failed to build HTTP client"),
    );

    let client = SearxngClient::new(http, &args.searxng_url, Duration::from_secs(args.timeout));

    let params = SearxngParams {
        q: args.query.clone(),
        categories: args.category,
        language: args.language,
        time_range: args.time_range,
        engines: None,
        pageno: None,
        safesearch: args.safesearch,
    };

    let response = match client.fetch(&params).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: search failed: {e}");
            eprintln!();
            eprintln!("hint: Make sure SearXNG is running at {}", args.searxng_url);
            eprintln!("      You can start it with: docker run -p 8080:8080 searxng/searxng");
            std::process::exit(1);
        }
    };

    // Transform to flat result format
    let results = transform_flat(&response, args.limit);

    match args.format {
        SearchFormat::Json => match serde_json::to_string_pretty(&results) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("error: failed to serialize JSON: {e}");
                std::process::exit(1);
            }
        },
        SearchFormat::Text => {
            if results.is_empty() {
                println!("No results found for: {}", args.query);
            } else {
                for result in &results {
                    println!("{}", result.title);
                    println!("  {}", result.url);
                    if !result.description.is_empty() {
                        // Truncate long descriptions
                        let truncated: String = result.description.chars().take(200).collect();
                        if truncated.len() < result.description.len() {
                            println!("  {}...", truncated);
                        } else {
                            println!("  {}", result.description);
                        }
                    }
                    println!();
                }
            }
        }
        SearchFormat::Markdown => {
            if results.is_empty() {
                println!("No results found for: {}", args.query);
            } else {
                println!("# Search results for: {}\n", args.query);
                for (i, result) in results.iter().enumerate() {
                    println!("{}. [{}]({})", i + 1, result.title, result.url);
                    if !result.description.is_empty() {
                        println!("   > {}", result.description);
                    }
                    println!();
                }
            }
        }
    }
}
