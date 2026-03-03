# Crates

crw is split into focused crates that can be used independently or together as libraries.

## crw-core

Core types, configuration, and error handling shared by all crw crates.

```bash
cargo add crw-core
```

```rust
use crw_core::{AppConfig, CrwError, CrwResult};
use crw_core::types::{OutputFormat, ScrapeData};

let config = AppConfig::load()?;
println!("Server port: {}", config.server.port);
```

## crw-renderer

HTTP fetcher and CDP-based headless browser rendering with automatic SPA detection.

```bash
cargo add crw-renderer                # HTTP only
cargo add crw-renderer --features cdp # HTTP + CDP rendering
```

```rust
use crw_renderer::FallbackRenderer;
use crw_core::config::RendererConfig;
use std::collections::HashMap;

let config = RendererConfig::default();
let renderer = FallbackRenderer::new(&config, "my-bot/1.0", None);

let result = renderer.fetch(
    "https://example.com",
    &HashMap::new(),
    None,  // render_js: None = auto-detect
    None,  // wait_for_ms
).await?;

println!("Status: {}, HTML length: {}", result.status_code, result.html.len());
```

## crw-extract

HTML content extraction — converts raw HTML to markdown, plain text, or cleaned HTML.

```bash
cargo add crw-extract
```

```rust
use crw_extract::extract;
use crw_core::types::OutputFormat;

let html = "<html><body><h1>Title</h1><p>Content here.</p></body></html>";
let data = extract(
    html,
    "https://example.com",
    200,
    None,               // rendered_with
    42,                 // elapsed_ms
    &[OutputFormat::Markdown, OutputFormat::PlainText],
    true,               // only_main_content
    &[],                // include_tags
    &[],                // exclude_tags
);

println!("{}", data.markdown.unwrap());
```

## crw-crawl

Async BFS web crawler with rate limiting, robots.txt compliance, and sitemap support.

```bash
cargo add crw-crawl
```

```rust
use crw_crawl::robots;

// Check robots.txt before crawling
let allowed = robots::is_allowed(
    "https://example.com/robots.txt",
    "https://example.com/page",
    "my-bot",
).await?;
```

## crw-server

Axum-based HTTP API server — Firecrawl-compatible REST endpoints and built-in MCP transport.

```bash
cargo add crw-server
```

```rust
use crw_server::app;
use crw_core::AppConfig;

let config = AppConfig::load()?;
let app = app::build_app(config).await;

let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
axum::serve(listener, app).await?;
```

## crw-mcp

MCP stdio proxy binary — connects AI assistants to a running crw server.

```bash
cargo install crw-mcp
```

This is a standalone binary, not a library. See [MCP Server](#mcp) for setup instructions.
