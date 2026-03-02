# CRW

Lightweight, Firecrawl-compatible web scraper. Single binary, ~3MB idle RAM, optional JS rendering via LightPanda sidecar.

**API-compatible with [Firecrawl](https://firecrawl.dev)** — drop-in replacement for self-hosted deployments.

## Table of Contents

- [Quick Start](#quick-start)
- [Installation](#installation)
- [Configuration](#configuration)
- [API Reference](#api-reference)
- [MCP Server](#mcp-server)
- [Docker](#docker)
- [Architecture](#architecture)

---

## Quick Start

```bash
# Build
cargo build --release --bin crw-server

# Run
./target/release/crw-server

# Scrape a page
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'
```

Response:

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

---

## Installation

### From Source

Requires Rust 1.83+.

```bash
git clone <repo-url>
cd crw

# HTTP-only (no JS rendering)
cargo build --release --bin crw-server

# With JS rendering support (CDP)
cargo build --release --bin crw-server --features crw-server/cdp

# MCP server (for Claude Code / Claude Desktop)
cargo build --release --bin crw-mcp
```

**Binaries produced:**

| Binary | Path | Description |
|--------|------|-------------|
| `crw-server` | `target/release/crw-server` | API server |
| `crw-mcp` | `target/release/crw-mcp` | MCP server for LLM tools |

### With Docker

```bash
docker compose up
```

This starts the API server on port 3000 with LightPanda JS rendering on port 9222.

---

## Configuration

CRW loads config from `config.default.toml`, then overrides with environment variables.

### Environment Variables

All env vars use the `CRW_` prefix with `__` as nested separator.

#### Server

| Variable | Default | Description |
|----------|---------|-------------|
| `CRW_SERVER__HOST` | `0.0.0.0` | Bind address |
| `CRW_SERVER__PORT` | `3000` | Bind port |
| `CRW_SERVER__REQUEST_TIMEOUT_SECS` | `60` | Request timeout |

#### Renderer

| Variable | Default | Description |
|----------|---------|-------------|
| `CRW_RENDERER__MODE` | `auto` | `auto`, `none`, or specific renderer |
| `CRW_RENDERER__PAGE_TIMEOUT_MS` | `30000` | JS page render timeout |
| `CRW_RENDERER__LIGHTPANDA__WS_URL` | — | LightPanda WebSocket URL |
| `CRW_RENDERER__PLAYWRIGHT__WS_URL` | — | Playwright WebSocket URL |
| `CRW_RENDERER__CHROME__WS_URL` | — | Chrome CDP WebSocket URL |

#### Crawler

| Variable | Default | Description |
|----------|---------|-------------|
| `CRW_CRAWLER__MAX_CONCURRENCY` | `10` | Max parallel requests |
| `CRW_CRAWLER__REQUESTS_PER_SECOND` | `10.0` | Rate limit |
| `CRW_CRAWLER__RESPECT_ROBOTS_TXT` | `true` | Honor robots.txt |
| `CRW_CRAWLER__USER_AGENT` | `CRW/0.1` | User-Agent string |
| `CRW_CRAWLER__DEFAULT_MAX_DEPTH` | `2` | Default crawl depth |
| `CRW_CRAWLER__DEFAULT_MAX_PAGES` | `100` | Default max pages |
| `CRW_CRAWLER__PROXY` | — | HTTP/HTTPS proxy URL |
| `CRW_CRAWLER__JOB_TTL_SECS` | `3600` | Crawl job cleanup TTL |

#### Extraction

| Variable | Default | Description |
|----------|---------|-------------|
| `CRW_EXTRACTION__DEFAULT_FORMAT` | `markdown` | Default output format |
| `CRW_EXTRACTION__ONLY_MAIN_CONTENT` | `true` | Strip nav/footer/sidebar |
| `CRW_EXTRACTION__LLM__PROVIDER` | `anthropic` | `anthropic` or `openai` |
| `CRW_EXTRACTION__LLM__API_KEY` | — | LLM API key |
| `CRW_EXTRACTION__LLM__MODEL` | `claude-sonnet-4-20250514` | Model name |
| `CRW_EXTRACTION__LLM__MAX_TOKENS` | `4096` | Max response tokens |
| `CRW_EXTRACTION__LLM__BASE_URL` | — | Custom API endpoint |

#### Auth

| Variable | Default | Description |
|----------|---------|-------------|
| `CRW_AUTH__API_KEYS` | `[]` | Bearer tokens (JSON array) |

### Config File

`config.default.toml`:

```toml
[server]
host = "0.0.0.0"
port = 3000
request_timeout_secs = 60

[renderer]
mode = "auto"                     # auto | none
page_timeout_ms = 30000

[renderer.lightpanda]
ws_url = "ws://127.0.0.1:9222"

[crawler]
max_concurrency = 10
requests_per_second = 10.0
respect_robots_txt = true
user_agent = "CRW/0.1"
default_max_depth = 2
default_max_pages = 100
job_ttl_secs = 3600
# proxy = "http://proxy:8080"

[extraction]
default_format = "markdown"
only_main_content = true

[auth]
# api_keys = ["your-api-key"]

# [extraction.llm]
# provider = "anthropic"
# api_key = "sk-..."
# model = "claude-sonnet-4-20250514"
# max_tokens = 4096
```

---

## API Reference

Base URL: `http://localhost:3000`

### Health Check

```
GET /health
```

No authentication required.

```bash
curl http://localhost:3000/health
```

```json
{
  "status": "ok",
  "version": "0.1.0",
  "renderers": {
    "http": true
  },
  "active_crawl_jobs": 0
}
```

---

### Scrape

```
POST /v1/scrape
```

Scrape a single URL and extract content.

**Request:**

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `url` | string | yes | — | URL to scrape (http/https only) |
| `formats` | string[] | no | `["markdown"]` | Output formats: `markdown`, `html`, `rawHtml`, `plainText`, `links`, `json` |
| `onlyMainContent` | boolean | no | `true` | Strip nav, footer, sidebar |
| `renderJs` | boolean | no | `null` | `null`=auto-detect, `true`=force JS, `false`=HTTP only |
| `waitFor` | number | no | — | Wait ms after JS render |
| `includeTags` | string[] | no | `[]` | CSS selectors to keep |
| `excludeTags` | string[] | no | `[]` | CSS selectors to remove |
| `headers` | object | no | `{}` | Custom request headers |
| `jsonSchema` | object | no | — | JSON Schema for LLM extraction (requires LLM config) |

**Examples:**

Basic scrape:

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'
```

```json
{
  "success": true,
  "data": {
    "markdown": "# Example Domain\nThis domain is for use in documentation examples without needing permission. Avoid use in operations.\n[Learn more](https://iana.org/domains/example)",
    "metadata": {
      "title": "Example Domain",
      "sourceURL": "https://example.com",
      "language": "en",
      "statusCode": 200,
      "elapsedMs": 32
    }
  }
}
```

Multiple formats:

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com",
    "formats": ["markdown", "html", "links"]
  }'
```

```json
{
  "success": true,
  "data": {
    "markdown": "# Example Domain\n...",
    "html": "<div><h1>Example Domain</h1>...</div>",
    "links": [
      "https://iana.org/domains/example"
    ],
    "metadata": {
      "title": "Example Domain",
      "sourceURL": "https://example.com",
      "language": "en",
      "statusCode": 200,
      "elapsedMs": 20
    }
  }
}
```

---

### Crawl

Start an async crawl job:

```
POST /v1/crawl
```

**Request:**

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `url` | string | yes | — | Starting URL |
| `maxDepth` | number | no | `2` | Max link depth |
| `maxPages` | number | no | `100` | Max pages to crawl |
| `formats` | string[] | no | `["markdown"]` | Output formats |
| `onlyMainContent` | boolean | no | `true` | Strip boilerplate |

```bash
curl -X POST http://localhost:3000/v1/crawl \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com",
    "maxDepth": 1,
    "maxPages": 2
  }'
```

```json
{
  "success": true,
  "id": "a4c03342-ab36-4df6-9e15-7ecffc9f8b3a"
}
```

Check crawl status:

```
GET /v1/crawl/:id
```

```bash
curl http://localhost:3000/v1/crawl/a4c03342-ab36-4df6-9e15-7ecffc9f8b3a
```

```json
{
  "status": "completed",
  "total": 1,
  "completed": 1,
  "data": [
    {
      "markdown": "# Example Domain\n...",
      "metadata": {
        "title": "Example Domain",
        "sourceURL": "https://example.com",
        "statusCode": 200,
        "elapsedMs": 12
      }
    }
  ]
}
```

Status values: `scraping`, `completed`, `failed`

---

### Map

```
POST /v1/map
```

Discover URLs on a site via crawling and sitemap.

**Request:**

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `url` | string | yes | — | URL to map |
| `maxDepth` | number | no | `2` | Discovery depth |
| `useSitemap` | boolean | no | `true` | Read sitemap.xml |

```bash
curl -X POST http://localhost:3000/v1/map \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'
```

```json
{
  "success": true,
  "links": [
    "https://example.com"
  ]
}
```

---

### Authentication

When `api_keys` is configured, all `/v1/*` endpoints require a Bearer token. The `/health` endpoint is always public.

```bash
# Start with auth
CRW_AUTH__API_KEYS='["sk-mykey123"]' ./target/release/crw-server

# Use with Bearer token
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer sk-mykey123" \
  -d '{"url": "https://example.com"}'
```

Without valid token:

```json
{
  "success": false,
  "error": "Missing Authorization header"
}
```

---

### Error Responses

All errors follow the same format:

```json
{
  "success": false,
  "error": "error message"
}
```

| HTTP Status | When |
|-------------|------|
| 400 | Invalid request (bad URL, missing fields) |
| 401 | Missing or invalid API key |
| 404 | Crawl job not found |
| 422 | LLM extraction failed |
| 502 | Upstream HTTP error |
| 504 | Request timeout |
| 500 | Internal error |

---

## MCP Server

CRW includes an MCP (Model Context Protocol) server so Claude Desktop and Claude Code can use it as a web scraping tool.

### Setup

Build:

```bash
cargo build --release --bin crw-mcp
```

Add to Claude Code (`~/.claude.json`):

```json
{
  "mcpServers": {
    "crw": {
      "command": "/absolute/path/to/crw-mcp",
      "env": {
        "CRW_API_URL": "http://localhost:3000"
      }
    }
  }
}
```

With authentication:

```json
{
  "mcpServers": {
    "crw": {
      "command": "/absolute/path/to/crw-mcp",
      "env": {
        "CRW_API_URL": "http://localhost:3000",
        "CRW_API_KEY": "sk-mykey123"
      }
    }
  }
}
```

For Claude Desktop, add the same config to `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS) or `%APPDATA%\Claude\claude_desktop_config.json` (Windows).

Restart Claude Code/Desktop after adding the config.

### MCP Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `CRW_API_URL` | `http://localhost:3000` | CRW server URL |
| `CRW_API_KEY` | — | Bearer token (if auth enabled) |

### Available Tools

After setup, these tools appear in Claude:

| Tool | Description |
|------|-------------|
| `crw_scrape` | Scrape a URL and return markdown/html/links |
| `crw_crawl` | Start an async crawl, returns job ID |
| `crw_check_crawl_status` | Poll crawl job status and get results |
| `crw_map` | Discover all URLs on a site |

### Verification

Test that MCP is working:

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"test"},"protocolVersion":"2024-11-05"}}' \
  | ./target/release/crw-mcp 2>/dev/null
```

Expected:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "protocolVersion": "2024-11-05",
    "capabilities": {"tools": {}},
    "serverInfo": {"name": "crw-mcp", "version": "0.1.0"}
  }
}
```

---

## Docker

### Docker Compose (recommended)

```bash
docker compose up
```

This starts:
- **crw** on port 3000 — API server with CDP enabled
- **lightpanda** on port 9222 — JS rendering sidecar

### Dockerfile

The multi-stage Dockerfile builds with CDP support and produces a minimal Debian-based image:

```bash
docker build -t crw .
docker run -p 3000:3000 crw
```

### Custom Environment

```bash
docker compose up -d

# Override settings
docker compose exec crw env CRW_CRAWLER__REQUESTS_PER_SECOND=5.0 crw-server
```

---

## JS Rendering

CRW supports JavaScript rendering for SPAs via CDP (Chrome DevTools Protocol) compatible browsers.

### LightPanda (recommended)

LightPanda is a lightweight headless browser (~3MB idle RAM).

```bash
# Start LightPanda
lightpanda serve --host 127.0.0.1 --port 9222

# Start CRW with CDP enabled
CRW_RENDERER__LIGHTPANDA__WS_URL=ws://127.0.0.1:9222 ./target/release/crw-server
```

> **Note:** The server binary must be built with the `cdp` feature: `cargo build --release --bin crw-server --features crw-server/cdp`

### Rendering Modes

| Mode | Behavior |
|------|----------|
| `auto` (default) | HTTP first, auto-detect SPA shells, retry with JS if needed |
| `none` | HTTP only, all JS renderers disabled |

Per-request control with the `renderJs` field:

| Value | Behavior |
|-------|----------|
| `null` (default) | Auto-detect: try HTTP, fallback to JS if SPA detected |
| `true` | Force JS rendering |
| `false` | HTTP only, skip JS |

---

## Architecture

```
crates/
  crw-core      Types, config (TOML + env), error types
  crw-extract   HTML cleaning, readability, markdown, plaintext, LLM extraction
  crw-renderer  PageFetcher trait, HTTP fetcher, CDP client (tokio-tungstenite)
  crw-crawl     Single URL scrape, BFS crawl, rate limiting, robots.txt, sitemap
  crw-server    Axum HTTP server, routes, auth middleware
  crw-mcp       MCP stdio server (JSON-RPC 2.0 proxy)
```

### Request Flow

```
Client → /v1/scrape → Validate URL
                     → Fetch (HTTP or JS)
                     → Clean HTML (lol_html)
                     → Extract main content
                     → Convert to requested formats
                     → Return JSON response
```

### Performance

| Metric | Value |
|--------|-------|
| Idle RAM | 3.3 MB (server) + 3.3 MB (LightPanda) |
| HTTP scrape | ~30ms avg |
| JS scrape | ~520ms avg |
| Cold start | ~85ms |

### Security

- SSRF protection: only `http://` and `https://` URLs allowed
- Response size limit: 10 MB
- Max crawl depth: 10, max pages: 1000
- Constant-time auth token comparison
- robots.txt respected by default

---

## License

MIT
