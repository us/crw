---
title: Home
layout: home
nav_order: 1
description: "CRW — lightweight, Firecrawl-compatible web scraper. Single binary, ~3MB RAM, JS rendering, MCP server for Claude."
permalink: /
---

# CRW
{: .fs-9 }

Lightweight, Firecrawl-compatible web scraper. Single binary, ~3MB idle RAM, optional JS rendering via LightPanda.
{: .fs-6 .fw-300 }

[Get Started]({% link getting-started.md %}){: .btn .btn-primary .fs-5 .mb-4 .mb-md-0 .mr-2 }
[API Reference]({% link api-reference.md %}){: .btn .fs-5 .mb-4 .mb-md-0 }

**English** | [中文]({% link zh-CN/index.md %})

---

## Why CRW?

CRW is a **drop-in replacement** for [Firecrawl](https://firecrawl.dev) that you can self-host. It's built in Rust for maximum performance with minimal resource usage.

| | CRW | Firecrawl |
|---|---|---|
| **Idle RAM** | 3.3 MB | ~500 MB+ |
| **Cold start** | 85ms | seconds |
| **HTTP scrape** | ~30ms | ~200ms+ |
| **Binary size** | ~8 MB | Node.js runtime |
| **Dependencies** | single binary | Node, Redis, etc. |
| **License** | MIT | AGPL |

## Features

- **Firecrawl-compatible API** — same endpoints, same request/response format
- **4 endpoints** — `/v1/scrape`, `/v1/crawl`, `/v1/crawl/:id`, `/v1/map`
- **Multiple output formats** — markdown, HTML, cleaned HTML, plain text, links
- **JS rendering** — auto-detect SPAs, render via LightPanda, Playwright, or Chrome
- **BFS crawler** — async crawl with rate limiting, robots.txt, sitemap support
- **LLM extraction** — structured JSON output via Claude or OpenAI
- **MCP server** — use CRW as a tool in Claude Code or Claude Desktop
- **Auth** — optional Bearer token authentication
- **Docker ready** — multi-stage Dockerfile + docker-compose with LightPanda

## Quick Example

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'
```

```json
{
  "success": true,
  "data": {
    "markdown": "# Example Domain\nThis domain is for use in ...",
    "metadata": {
      "title": "Example Domain",
      "sourceURL": "https://example.com",
      "statusCode": 200,
      "elapsedMs": 32
    }
  }
}
```

## Architecture

```
crates/
  crw-core      Types, config, error types
  crw-extract   HTML cleaning, readability, markdown, LLM extraction
  crw-renderer  HTTP fetcher, CDP client (tokio-tungstenite)
  crw-crawl     Single scrape, BFS crawl, rate limiting, robots.txt
  crw-server    Axum HTTP server, routes, auth middleware
  crw-mcp       MCP stdio server (JSON-RPC 2.0 proxy)
```
