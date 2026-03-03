---
title: Home
layout: home
nav_order: 1
description: "CRW — lightweight, Firecrawl-compatible web scraper and crawler for AI. Single binary, ~3MB RAM, LLM extraction, MCP server for Claude, JS rendering."
permalink: /
---

# CRW
{: .fs-9 }

Lightweight, Firecrawl-compatible web scraper and crawler for AI. Single binary, ~3 MB idle RAM, LLM structured extraction, MCP server for Claude.
{: .fs-6 .fw-300 }

[Get Started]({% link getting-started.md %}){: .btn .btn-primary .fs-5 .mb-4 .mb-md-0 .mr-2 }
[API Reference]({% link api-reference.md %}){: .btn .fs-5 .mb-4 .mb-md-0 }

**English** | [中文]({% link zh-CN/index.md %})

---

## Why CRW?

CRW is a **drop-in replacement** for [Firecrawl](https://firecrawl.dev) that you can self-host. Built in Rust for maximum performance with minimal resource usage — no Node.js, no Redis, just a single binary.

| | CRW | Firecrawl |
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
| **License** | MIT | AGPL |

Benchmark: [Firecrawl scrape-content-dataset-v1](https://huggingface.co/datasets/firecrawl/scrape-content-dataset-v1) — 1,000 real-world URLs.

## Features

- **🔌 Firecrawl-compatible API** — same endpoints, same request/response format, drop-in replacement
- **📄 6 output formats** — markdown, HTML, cleaned HTML, raw HTML, plain text, links, structured JSON
- **🤖 LLM structured extraction** — send a JSON schema, get validated structured data back (Anthropic tool_use + OpenAI function calling)
- **🌐 JS rendering** — auto-detect SPAs with shell heuristics, render via LightPanda, Playwright, or Chrome (CDP)
- **🕷️ BFS crawler** — async crawl with rate limiting, robots.txt, sitemap support, concurrent jobs
- **🔧 MCP server** — built-in stdio + HTTP transport for Claude Code and Claude Desktop
- **🔒 Security** — SSRF protection (private IPs, cloud metadata, IPv6), constant-time auth, dangerous URI filtering
- **🐳 Docker ready** — multi-stage build with LightPanda sidecar

## Use Cases

- **RAG pipelines** — crawl websites and extract structured data for vector databases
- **AI agents** — give Claude Code or Claude Desktop web scraping tools via MCP
- **Content monitoring** — periodic crawl with LLM extraction to track changes
- **Data extraction** — combine CSS selectors + LLM to extract any schema from any page
- **Web archiving** — full-site BFS crawl to markdown

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
