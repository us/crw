# Introduction

CRW is an open-source, lightweight, self-hosted web scraper and web crawler written in Rust. A **drop-in replacement** for [Firecrawl](https://firecrawl.dev) and a faster alternative to [Crawl4AI](https://github.com/unclecode/crawl4ai) that you can self-host — no Node.js, no Redis, no Python, just a single binary. Built for AI agents, RAG pipelines, and LLM structured extraction with a built-in MCP server for Claude Code, Cursor, Windsurf, and 8+ other platforms.

## Benchmarks

Tested against [Firecrawl scrape-content-dataset-v1](https://huggingface.co/datasets/firecrawl/scrape-content-dataset-v1) — 1,000 real-world URLs:

| | CRW (self-hosted) | fastcrw.com (cloud) | Firecrawl | Crawl4AI | Spider |
|---|---|---|---|---|---|
| **Coverage (1K URLs)** | **92.0%** | **92.0%** | 77.2% | — | 99.9% |
| **Avg Latency** | **833ms** | **833ms** | 4,600ms | — | — |
| **P50 Latency** | **446ms** | **446ms** | — | — | 45ms (static) |
| **Noise Rejection** | **88.4%** | **88.4%** | noise 6.8% | noise 11.3% | noise 4.2% |
| **Idle RAM** | 6.6 MB | 0 (managed) | ~500 MB+ | — | cloud-only |
| **Cold start** | 85 ms | 0 (always-on) | 30–60 s | — | — |
| **HTTP scrape** | ~30 ms | ~30 ms | ~200 ms+ | ~480 ms | ~45 ms |
| **Proxy network** | BYO | Global (built-in) | Built-in | — | Cloud-only |
| **Cost / 1K scrapes** | **$0** (self-hosted) | From $13/mo | $0.83–5.33 | $0 | $0.65 |
| **Dependencies** | single binary | None (API) | Node + Redis + PG + RabbitMQ | Python + Playwright | Rust / cloud |

### How crw compares

**vs Firecrawl** — crw covers 15% more URLs (92% vs 77.2%), runs 5.5x faster on average, and uses ~75x less RAM at idle. Firecrawl requires 5 containers (Node.js, Redis, PostgreSQL, RabbitMQ, Playwright); crw is a single binary. Firecrawl's [independent Scrapeway benchmark](https://scrapeway.com/web-scraping-api/firecrawl) shows 64.3% success rate and $5.11/1K cost, with 0% success on LinkedIn/Twitter.

**vs Crawl4AI** — Both are free and self-hosted. Crawl4AI is Python-based and depends on Playwright (~200 MB RAM per browser). crw ships as a single binary with optional LightPanda sidecar (~3.3 MB idle). In [Spider.cloud's benchmark](https://spider.cloud/blog/firecrawl-vs-crawl4ai-vs-spider-honest-benchmark), Crawl4AI showed 19 pages/sec throughput, 11.3% noise ratio, and 72% anti-bot success — while crw achieves 187+ pages/sec throughput with 88.4% noise rejection.

**vs Spider** — Spider-RS is the fastest crawler in raw throughput (182 pages/sec static, 99.9% coverage). However, Spider's advanced features (anti-bot, proxy rotation) require their paid cloud service. crw offers a Firecrawl-compatible API (drop-in replacement), built-in MCP server for AI agents, and LLM structured extraction — features Spider doesn't provide out of the box.

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
