<p align="center">
  <h1 align="center">CRW</h1>
  <p align="center">Lightweight, Firecrawl-compatible web scraper and crawler for AI</p>
  <p align="center">
    <a href="docs/docs/installation.md">Installation</a> &bull;
    <a href="docs/docs/rest-api.md">API Reference</a> &bull;
    <a href="docs/docs/mcp.md">MCP Integration</a> &bull;
    <a href="docs/docs/js-rendering.md">JS Rendering</a> &bull;
    <a href="docs/docs/configuration.md">Configuration</a>
  </p>
  <p align="center">
    <b>English</b> | <a href="README.zh-CN.md">中文</a>
  </p>
</p>

---

CRW is a self-hosted web scraper and crawler built in Rust — a drop-in Firecrawl replacement designed for LLM extraction, RAG pipelines, and AI agents. Single binary, ~6 MB idle RAM, built-in MCP server for Claude, structured data extraction via Anthropic and OpenAI.

**Single binary. No Redis. No Node.js. Drop-in Firecrawl API.**

```bash
cargo install crw-server
crw-server
```

## What's New

### `crw-server setup` Command

- **One-command JS rendering setup** — `crw-server setup` downloads LightPanda and creates `config.local.toml` automatically
- **Platform detection** — detects OS/arch and downloads the correct binary (Linux x86_64, macOS aarch64)
- **CLI subcommands** — crw-server now uses clap for extensible subcommand support

### v0.0.1

- **Firecrawl-compatible REST API** — `/v1/scrape`, `/v1/crawl`, `/v1/map` with identical request/response format
- **6 output formats** — markdown, HTML, cleaned HTML, raw HTML, plain text, links, structured JSON
- **LLM structured extraction** — JSON schema in, validated structured data out (Anthropic tool_use + OpenAI function calling)
- **JS rendering** — auto-detect SPAs via heuristics, render via LightPanda, Playwright, or Chrome (CDP)
- **BFS crawler** — async crawl with rate limiting, robots.txt, sitemap support, concurrent jobs
- **MCP server** — built-in stdio + HTTP transport for Claude Code and Claude Desktop
- **SSRF protection** — private IPs, cloud metadata, IPv6, dangerous URI filtering
- **Docker ready** — multi-stage build with LightPanda sidecar

## Why CRW?

CRW gives you Firecrawl's API with a fraction of the resource usage. No runtime dependencies, no Redis, no Node.js — just a single binary you can deploy anywhere.

| | **CRW** | Firecrawl |
|---|---|---|
| **Coverage (1K URLs)** | **92.0%** | 77.2% |
| **Avg Latency** | **833ms** | 4,600ms |
| **P50 Latency** | **446ms** | — |
| **Noise Rejection** | **88.4%** | — |
| **Idle RAM** | **6.6 MB** | ~500 MB+ |
| **Cold start** | **85 ms** | seconds |
| **HTTP scrape** | **~30 ms** | ~200 ms+ |
| **Binary size** | **~8 MB** | Node.js runtime |
| **Cost / 1K scrapes** | **$0** (self-hosted) | $0.83–5.33 |
| **Dependencies** | single binary | Node + Redis |

Benchmark: [Firecrawl scrape-content-dataset-v1](https://huggingface.co/datasets/firecrawl/scrape-content-dataset-v1) — 1,000 real-world URLs with JS rendering enabled.

## Quick Start

### Install and Run

```bash
cargo install crw-server
crw-server
# Server starts on http://localhost:3000
```

### Enable JS Rendering (Optional)

```bash
crw-server setup
lightpanda serve --host 127.0.0.1 --port 9222 &
crw-server
```

### Docker

```bash
# Pre-built image
docker run -p 3000:3000 ghcr.io/us/crw:latest

# With JS rendering sidecar
docker compose up
```

### Scrape a Page

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

### Crawl a Site

```bash
# Start async crawl
curl -X POST http://localhost:3000/v1/crawl \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com", "maxDepth": 2, "maxPages": 50}'

# Check status
curl http://localhost:3000/v1/crawl/<job-id>
```

### LLM Structured Extraction

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com/product",
    "formats": ["json"],
    "jsonSchema": {
      "type": "object",
      "properties": {
        "name": { "type": "string" },
        "price": { "type": "number" }
      },
      "required": ["name", "price"]
    }
  }'
```

Configure the provider:

```toml
[extraction.llm]
provider = "anthropic"        # "anthropic" or "openai"
api_key = "sk-..."            # or CRW_EXTRACTION__LLM__API_KEY env var
model = "claude-sonnet-4-20250514"
```

### Use with MCP (Claude Code, Cursor)

```bash
# HTTP transport (recommended)
claude mcp add --transport http crw http://localhost:3000/mcp

# Stdio transport
cargo install crw-mcp
```

Add to your AI tool's MCP config:

```json
{
  "mcpServers": {
    "crw": {
      "command": "crw-mcp",
      "env": { "CRW_API_URL": "http://localhost:3000" }
    }
  }
}
```

Tools: `crw_scrape`, `crw_crawl`, `crw_check_crawl_status`, `crw_map`

See the full [MCP setup guide](docs/docs/mcp.md) for Claude Desktop, Cursor, Windsurf, Cline, Continue.dev, and Codex CLI.

## Features

| Feature | Description |
|---------|-------------|
| **Firecrawl API** | Drop-in compatible `/v1/scrape`, `/v1/crawl`, `/v1/map` endpoints |
| **6 Output Formats** | Markdown, HTML, cleaned HTML, raw HTML, plain text, links, structured JSON |
| **LLM Extraction** | Send JSON schema, get validated structured data (Anthropic + OpenAI) |
| **JS Rendering** | Auto-detect SPAs, render via LightPanda, Playwright, or Chrome (CDP) |
| **BFS Crawler** | Async crawl with rate limiting, robots.txt, sitemap, concurrent jobs |
| **MCP Server** | Built-in stdio + HTTP transport for Claude Code and Claude Desktop |
| **One-Command Setup** | `crw-server setup` downloads LightPanda and creates config |
| **SSRF Protection** | Private IPs, cloud metadata, IPv6, dangerous URI filtering |
| **Auth** | Optional Bearer token with constant-time comparison |
| **Docker** | Multi-stage build with LightPanda sidecar |

## Security

CRW includes built-in protections against common web scraping attack vectors:

- **SSRF protection** — all URL inputs (REST API + MCP) are validated against private/internal networks:
  - Loopback (`127.0.0.0/8`, `::1`, `localhost`)
  - Private IPs (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`)
  - Link-local / cloud metadata (`169.254.0.0/16` — blocks AWS/GCP metadata endpoints)
  - IPv6 mapped addresses (`::ffff:127.0.0.1`), link-local (`fe80::`), ULA (`fc00::/7`)
  - Non-HTTP schemes (`file://`, `ftp://`, `gopher://`, `data:`)
- **Auth** — optional Bearer token with constant-time comparison (no length or key-index leakage)
- **robots.txt** — respects `Allow`/`Disallow` with wildcard patterns (`*`, `$`) and RFC 9309 specificity
- **Rate limiting** — configurable per-second request cap
- **Resource limits** — max body size (1 MB), max crawl depth (10), max pages (1000), max discovered URLs (5000)

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

## Configuration

CRW uses layered TOML configuration with environment variable overrides:

1. `config.default.toml` — built-in defaults
2. `config.local.toml` — local overrides (or set `CRW_CONFIG=myconfig`)
3. Environment variables — `CRW_` prefix with `__` separator (e.g. `CRW_SERVER__PORT=8080`)

```toml
[server]
host = "0.0.0.0"
port = 3000

[renderer]
mode = "auto"  # auto | lightpanda | playwright | chrome | none

[crawler]
max_concurrency = 10
requests_per_second = 10.0
respect_robots_txt = true

[auth]
# api_keys = ["fc-key-1234"]
```

See [Configuration Guide](docs/docs/configuration.md) for all options.

## Crates

| Crate | Description | |
|-------|-------------|-|
| [`crw-core`](crates/crw-core) | Core types, config, and error handling | [![crates.io](https://img.shields.io/crates/v/crw-core.svg)](https://crates.io/crates/crw-core) |
| [`crw-renderer`](crates/crw-renderer) | HTTP + CDP browser rendering engine | [![crates.io](https://img.shields.io/crates/v/crw-renderer.svg)](https://crates.io/crates/crw-renderer) |
| [`crw-extract`](crates/crw-extract) | HTML → markdown/plaintext extraction | [![crates.io](https://img.shields.io/crates/v/crw-extract.svg)](https://crates.io/crates/crw-extract) |
| [`crw-crawl`](crates/crw-crawl) | Async BFS crawler with robots.txt & sitemap | [![crates.io](https://img.shields.io/crates/v/crw-crawl.svg)](https://crates.io/crates/crw-crawl) |
| [`crw-server`](crates/crw-server) | Axum API server (Firecrawl-compatible) | [![crates.io](https://img.shields.io/crates/v/crw-server.svg)](https://crates.io/crates/crw-server) |
| [`crw-mcp`](crates/crw-mcp) | MCP stdio proxy binary | [![crates.io](https://img.shields.io/crates/v/crw-mcp.svg)](https://crates.io/crates/crw-mcp) |

## Documentation

- [Installation](docs/docs/installation.md) — Install from crates.io, source, or Docker
- [Quick Start](docs/docs/quick-start.md) — First scrape in 30 seconds
- [REST API](docs/docs/rest-api.md) — Complete endpoint reference
- [Scraping](docs/docs/scraping.md) — Output formats, selectors, LLM extraction
- [Crawling](docs/docs/crawling.md) — BFS crawler, depth/page limits, sitemap
- [JS Rendering](docs/docs/js-rendering.md) — LightPanda, Playwright, Chrome setup
- [MCP Integration](docs/docs/mcp.md) — Claude Code, Cursor, Windsurf, and more
- [Configuration](docs/docs/configuration.md) — All config options explained
- [Docker](docs/docs/docker.md) — Container deployment with sidecar
- [Architecture](docs/docs/architecture.md) — Internal design and crate structure

## Contributing

Contributions are welcome! Please open an issue or submit a pull request.

```bash
git clone https://github.com/us/crw
cd crw
cargo build --release
cargo test --workspace
```

## License

AGPL-3.0 — See [LICENSE](LICENSE) for details.
