# Introduction

crw is a lightweight, self-hosted web scraper and crawler written in Rust. A **drop-in replacement** for [Firecrawl](https://firecrawl.dev) that you can self-host — no Node.js, no Redis, just a single binary.

## Benchmarks

Tested against [Firecrawl scrape-content-dataset-v1](https://huggingface.co/datasets/firecrawl/scrape-content-dataset-v1) — 1,000 real-world URLs:

| | crw | Firecrawl |
|---|---|---|
| **Coverage (1K URLs)** | **92.0%** | 77.2% |
| **Avg Latency** | **833ms** | 4,600ms |
| **P50 Latency** | **446ms** | — |
| **Noise Rejection** | **88.4%** | — |
| **Idle RAM** | 6.6 MB | ~500 MB+ |
| **Cold start** | 85 ms | seconds |
| **HTTP scrape** | ~30 ms | ~200 ms+ |
| **Binary size** | ~8 MB | Node.js runtime |
| **Cost / 1K scrapes** | **$0** | $0.83–5.33 |
| **Dependencies** | single binary | Node + Redis |

## Features

- **Firecrawl-compatible API** — Same endpoints, same request/response format, drop-in replacement
- **6 output formats** — Markdown, HTML, cleaned HTML, raw HTML, plain text, links, structured JSON
- **LLM structured extraction** — Send a JSON schema, get validated structured data back (Anthropic tool_use + OpenAI function calling)
- **JS rendering** — Auto-detect SPAs with shell heuristics, render via LightPanda, Playwright, or Chrome (CDP)
- **BFS crawler** — Async crawl with rate limiting, robots.txt, sitemap support, concurrent jobs
- **MCP server** — Built-in stdio + HTTP transport for Claude Code, Cursor, and 8+ other platforms
- **Security** — SSRF protection (private IPs, cloud metadata, IPv6), constant-time auth, dangerous URI filtering
- **Docker ready** — Multi-stage build with LightPanda sidecar

## Use Cases

- **RAG pipelines** — Crawl websites and extract structured data for vector databases
- **AI agents** — Give Claude Code or Claude Desktop web scraping tools via MCP
- **Content monitoring** — Periodic crawl with LLM extraction to track changes
- **Data extraction** — Combine CSS selectors + LLM to extract any schema from any page
- **Web archiving** — Full-site BFS crawl to markdown

## Architecture

```
┌─────────────────────────────────────────────┐
│                 crw-server                  │
│         Axum HTTP API + Auth + MCP          │
├──────────┬──────────┬───────────────────────┤
│ crw-crawl│crw-extract│    crw-renderer      │
│ BFS crawl│ HTML→MD   │  HTTP + CDP(WS)      │
│ robots   │ LLM/JSON  │  LightPanda/Chrome   │
│ sitemap  │ clean/read│  auto-detect SPA     │
├──────────┴──────────┴───────────────────────┤
│                 crw-core                    │
│        Types, Config, Errors                │
└─────────────────────────────────────────────┘
```

## Security

| Layer | Protection |
|-------|-----------|
| SSRF | Blocks loopback, private IPs (10.x, 172.16-31.x, 192.168.x), link-local (169.254.x), IPv6 ULA/link-local, IPv4-mapped IPv6, non-HTTP schemes |
| Auth | Constant-time Bearer token comparison, multiple API keys |
| Rate limiting | Configurable requests-per-second for crawling |
| Resource limits | 1 MB request body, 10 MB HTTP response, max depth 10, max pages 1000 |
| Link filtering | Removes `javascript:`, `mailto:`, `data:`, `tel:`, `blob:` URIs |
| robots.txt | RFC 9309 compliant with specificity-based matching |
