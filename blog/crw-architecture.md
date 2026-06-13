# Inside CRW: Architecture of a Lightweight Rust Scraping API

> A technical deep-dive into CRW's Axum-based API, lol-html parser, LightPanda integration, and how it stays a small single static binary with a tiny idle footprint.

**Published:** 2026-04-12  
**Updated:** 2026-04-12  
**Canonical:** https://fastcrw.com/blog/crw-architecture

---

## Overview

CRW is a web scraping and crawling API written in Rust. It exposes a Firecrawl-compatible REST interface and ships as a single binary. This post explains the architectural decisions behind it — what we chose, why, and where the tradeoffs lie.

## HTTP Layer: Axum

CRW uses [Axum](https://github.com/tokio-rs/axum) as its HTTP framework. Axum is built on Tokio (Rust's async runtime) and Tower (the middleware framework). We chose it over alternatives (Actix-web, Warp) for three reasons:

- **Type safety:** Axum's extractor system makes request/response handling explicit and compile-time checked
- **Ecosystem alignment:** Tower middleware integrates directly, enabling rate limiting, timeout handling, and tracing with minimal code
- **Ergonomics:** Handler signatures in Axum are natural Rust functions with dependency injection via extractors

A simplified view of the route structure:

```
let app = Router::new()
    .route("/v1/scrape", post(scrape_handler))
    .route("/v1/crawl", post(crawl_start_handler))
    .route("/v1/crawl/:id", get(crawl_status_handler))
    .route("/v1/map", post(map_handler))
    .route("/health", get(health_handler))
    .layer(AuthLayer::new(config.api_key.clone()))
    .layer(RateLimitLayer::new(config.rate_limit))
    .layer(TraceLayer::new_for_http());
```

## HTML Parsing: lol-html

The most important architectural decision in CRW is the choice of HTML parser. Instead of building or wrapping a full DOM tree, CRW uses [lol-html](https://github.com/cloudflare/lol-html) — Cloudflare's streaming HTML rewriter.

lol-html processes HTML in a single linear pass. It never builds a full in-memory DOM tree. Instead, it operates on a streaming model where you register handlers for specific CSS selectors, and those handlers fire as matching elements are encountered during the parse.

```
let mut output = Vec::new();
let mut rewriter = HtmlRewriter::new(
    Settings {
        element_content_handlers: vec![
            element!("nav, footer, aside, .ad, script, style", |el| {
                el.remove();
                Ok(())
            }),
            // Text handler captures all text nodes for content extraction
            text!("article, main, .content", |text| {
                output.push_str(text.as_str());
                Ok(())
            }),
        ],
        ..Settings::default()
    },
    |c: &[u8]| { /* write output */ },
);
rewriter.write(html.as_bytes())?;
```

This approach has two key properties:

1. **Memory efficiency:** Memory usage is proportional to the largest element encountered, not the entire page. A 100 KB page doesn't require 100 KB of DOM tree in memory.
2. **Speed:** A single sequential pass is cache-friendly and avoids the overhead of building and traversing a tree data structure.

The tradeoff: you lose the ability to do backward references or complex DOM queries. But for content extraction (remove noise, extract text, convert to markdown), forward-only processing is sufficient.

## Markdown Conversion

After lol-html extracts clean HTML content, CRW converts it to markdown. We use a custom converter rather than an off-the-shelf HTML-to-markdown library because the quality requirements for LLM consumption are specific:

- Preserve code block fencing with language hints
- Convert tables to pipe-format markdown
- Preserve heading hierarchy
- Collapse excessive whitespace and blank lines
- Handle nested lists correctly
- Strip image alt text that doesn't add meaning

The converter is a stack-based state machine that walks the extracted HTML tree and emits markdown tokens. Output is normalized before returning — consistent indentation, no trailing spaces, maximum two consecutive blank lines.

## HTTP Client: reqwest

Outbound HTTP requests use reqwest with a connection pool. Key configuration:

- **Pool size:** 10 idle connections per host, 100 total
- **Timeout:** 30 seconds default, configurable per-request
- **User-agent:** Realistic browser UA strings with rotation
- **Redirect policy:** Follow up to 10 redirects
- **Cookie store:** Session-scoped per crawl job

For concurrent crawl jobs, each job gets its own client instance with an isolated cookie jar, preventing session contamination across jobs.

## JavaScript Rendering: LightPanda Integration

For JavaScript-heavy pages that require browser execution, CRW integrates with [LightPanda](https://github.com/lightpanda-io/browser) — an experimental browser written in Zig, designed for headless automation with a smaller footprint than Chromium.

The integration uses a sidecar pattern:

```
// When JS rendering is requested
if request.requires_js_render() {
    let result = lightpanda_client
        .render(url, timeout)
        .await?;
    process_html(result.html)
} else {
    // Fast path: direct HTTP + lol-html
    let html = http_client.get(url).await?;
    process_html(html)
}
```

LightPanda is launched on-demand and communicates over a local TCP socket. It's not pre-loaded, which is why the idle footprint stays tiny — LightPanda only consumes memory when actually needed.

The honest caveat: LightPanda is newer and less complete than Chromium. For complex SPAs with heavy client-side routing, it's functional but occasionally less reliable. This is the area of the architecture most actively under development.

## Crawl Orchestration

Multi-page crawls are managed by an async task graph:

1. **Seed URL:** The starting URL is added to a work queue
2. **Parallel workers:** N async workers dequeue URLs, scrape them, extract links
3. **Deduplication:** A bloom filter tracks visited URLs to avoid re-crawling
4. **Limit control:** Workers respect the `limit` parameter (the maximum number of pages to crawl)
5. **Robots.txt:** Robots.txt is fetched and honored by default
6. **Result aggregation:** Scraped pages are accumulated and returned when the job completes

Crawl state is held in memory (not persisted). This means a CRW restart abandons in-progress crawls. For large-scale crawling requiring durability, a queuing layer (Redis, SQS) should be added externally.

## MCP Server

The MCP (Model Context Protocol) server reuses CRW's scraping engine directly. It exposes six tools:

- `crw_scrape` — delegates to the scrape handler
- `crw_crawl` — starts an async crawl job and returns its id; the caller polls `crw_check_crawl_status` until the job completes (the same async model as the HTTP `/v1/crawl` endpoint)
- `crw_check_crawl_status` — polls a crawl job and returns its pages
- `crw_map` — returns the URL map for a site
- `crw_search` — web search (always in proxy mode; in embedded mode only with a SearXNG backend)
- `crw_parse_file` — parses a local PDF to markdown

MCP uses stdio transport (JSON-RPC over stdin/stdout), making it suitable for process-spawning clients like Claude Desktop. The server shares the same Tokio runtime as the HTTP API, so both can run simultaneously if needed.

## LLM Extraction

Requesting `formats: ["json"]` with a top-level `jsonSchema` object calls an LLM to extract structured data from the scraped content. The flow:

1. Scrape the page to clean markdown
2. Construct a prompt: *"Extract the following fields from this content: {jsonSchema}"*
3. Send to the configured LLM (OpenAI by default, configurable)
4. Parse and validate the LLM response against the JSON schema
5. Return the structured object at `data.json`

LLM calls are the most expensive operation in CRW — both in latency and token cost. They're only triggered when the caller requests the `json` format explicitly (billed at 5 credits per extraction).

## Why CRW Uses So Little Memory

The tiny idle footprint comes from:

- **No garbage collector:** Rust's ownership model means memory is freed deterministically when objects go out of scope. No heap bloat from GC pressure.
- **No JVM/V8 heap:** No runtime to warm up, no compiled bytecode cache.
- **Streaming parser:** lol-html doesn't hold full page content in memory.
- **On-demand browser:** LightPanda only starts when needed.
- **Minimal base dependencies:** The Axum + Tokio stack is lean by design.

Under load, memory grows with active request state — connection buffers, in-progress parse state, crawl job queues. But this growth is proportional to actual work, not baseline overhead.

## What We'd Do Differently

Some architectural decisions we'd revisit with hindsight:

**Crawl state persistence:** Storing crawl state in memory means restarts abandon jobs. Adding optional Redis-backed state would improve reliability for long crawls without significantly increasing deployment complexity.

**LightPanda coupling:** The dependency on LightPanda for JS rendering creates an external process dependency. A tighter integration or an alternative browser backend would improve reliability for SPA-heavy workloads.

**Plugin system:** For teams that need custom extraction logic, the current architecture provides no extension point. A WASM plugin system could enable custom extraction without sacrificing the single-binary model.
