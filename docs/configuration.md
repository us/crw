---
title: Configuration
layout: default
nav_order: 3
description: "Configure CRW web scraper: environment variables, config file, renderer setup, auth, LLM extraction."
---

# Configuration
{: .no_toc }

CRW loads config from `config.default.toml`, then overrides with environment variables.
{: .fs-6 .fw-300 }

## Table of Contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Environment Variables

All env vars use the `CRW_` prefix with `__` as nested separator.

```bash
# Example: change port to 8080
CRW_SERVER__PORT=8080 ./target/release/crw-server
```

### Server

| Variable | Default | Description |
|:---------|:--------|:------------|
| `CRW_SERVER__HOST` | `0.0.0.0` | Bind address |
| `CRW_SERVER__PORT` | `3000` | Bind port |
| `CRW_SERVER__REQUEST_TIMEOUT_SECS` | `60` | Global request timeout in seconds |

### Renderer

| Variable | Default | Description |
|:---------|:--------|:------------|
| `CRW_RENDERER__MODE` | `auto` | Renderer mode: `auto` or `none` |
| `CRW_RENDERER__PAGE_TIMEOUT_MS` | `30000` | JS page render timeout in ms |
| `CRW_RENDERER__LIGHTPANDA__WS_URL` | — | LightPanda CDP WebSocket URL |
| `CRW_RENDERER__PLAYWRIGHT__WS_URL` | — | Playwright CDP WebSocket URL |
| `CRW_RENDERER__CHROME__WS_URL` | — | Chrome/Chromium CDP WebSocket URL |

### Crawler

| Variable | Default | Description |
|:---------|:--------|:------------|
| `CRW_CRAWLER__MAX_CONCURRENCY` | `10` | Max parallel crawl requests |
| `CRW_CRAWLER__REQUESTS_PER_SECOND` | `10.0` | Rate limit (token bucket) |
| `CRW_CRAWLER__RESPECT_ROBOTS_TXT` | `true` | Honor robots.txt directives |
| `CRW_CRAWLER__USER_AGENT` | `CRW/0.1` | User-Agent header string |
| `CRW_CRAWLER__DEFAULT_MAX_DEPTH` | `2` | Default crawl depth limit |
| `CRW_CRAWLER__DEFAULT_MAX_PAGES` | `100` | Default max pages per crawl |
| `CRW_CRAWLER__PROXY` | — | HTTP/HTTPS proxy URL |
| `CRW_CRAWLER__JOB_TTL_SECS` | `3600` | Completed job cleanup TTL |

### Extraction

| Variable | Default | Description |
|:---------|:--------|:------------|
| `CRW_EXTRACTION__DEFAULT_FORMAT` | `markdown` | Default output format |
| `CRW_EXTRACTION__ONLY_MAIN_CONTENT` | `true` | Strip nav, footer, sidebar |

### LLM Extraction

| Variable | Default | Description |
|:---------|:--------|:------------|
| `CRW_EXTRACTION__LLM__PROVIDER` | `anthropic` | `anthropic` or `openai` |
| `CRW_EXTRACTION__LLM__API_KEY` | — | LLM API key (required for JSON extraction) |
| `CRW_EXTRACTION__LLM__MODEL` | `claude-sonnet-4-20250514` | Model name |
| `CRW_EXTRACTION__LLM__MAX_TOKENS` | `4096` | Max response tokens |
| `CRW_EXTRACTION__LLM__BASE_URL` | — | Custom API endpoint (OpenAI-compatible) |

### Authentication

| Variable | Default | Description |
|:---------|:--------|:------------|
| `CRW_AUTH__API_KEYS` | `[]` | JSON array of valid Bearer tokens |

---

## Config File

Default config at `config.default.toml`:

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

# [renderer.playwright]
# ws_url = "ws://playwright:9222"

# [renderer.chrome]
# ws_url = "ws://chrome:9222"

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
# base_url = "https://custom-endpoint.example.com"
```

{: .note }
Environment variables always override config file values. This makes it easy to use the same config file across environments while customizing via env vars in production.

---

## Authentication

When `api_keys` is set, all `/v1/*` endpoints require a valid Bearer token. The `/health` endpoint is always public.

```bash
# Enable auth with one or more keys
CRW_AUTH__API_KEYS='["sk-key-1", "sk-key-2"]' ./target/release/crw-server
```

Clients must send the token in the `Authorization` header:

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer sk-key-1" \
  -d '{"url": "https://example.com"}'
```

Without a valid token:

```json
{
  "success": false,
  "error": "Missing Authorization header"
}
```

{: .note }
Token comparison uses constant-time equality to prevent timing attacks.

---

## JS Rendering

CRW supports JavaScript rendering for SPAs via CDP (Chrome DevTools Protocol).

### Supported Renderers

| Renderer | Description | Idle RAM |
|:---------|:-----------|:---------|
| [LightPanda](https://github.com/nicholasgasior/lightpanda) | Lightweight headless browser | ~3 MB |
| Playwright | Full browser automation | ~200 MB |
| Chrome/Chromium | Standard headless Chrome | ~150 MB |

### Setup with LightPanda

```bash
# Start LightPanda
lightpanda serve --host 127.0.0.1 --port 9222

# Start CRW (must be built with cdp feature)
CRW_RENDERER__LIGHTPANDA__WS_URL=ws://127.0.0.1:9222 ./target/release/crw-server
```

{: .warning }
The server binary must be built with the `cdp` feature: `cargo build --release --bin crw-server --features crw-server/cdp`

### Rendering Modes

**Server-level** (config):

| Mode | Behavior |
|:-----|:---------|
| `auto` (default) | HTTP first, auto-detect SPA shells, retry with JS if needed |
| `none` | HTTP only, all JS renderers disabled |

**Per-request** (`renderJs` field):

| Value | Behavior |
|:------|:---------|
| `null` (default) | Auto-detect: try HTTP, fallback to JS if SPA detected |
| `true` | Force JS rendering |
| `false` | HTTP only, skip JS |

### SPA Detection

In `auto` mode, CRW detects SPA shells by looking for:
- Empty `<body>` or minimal content with `<div id="root">`
- Framework markers: React, Vue, Angular, Svelte
- Noscript tags suggesting JS is required

When detected, CRW automatically retries with a JS renderer.

---

## LLM Extraction

Extract structured JSON from web pages using Claude or OpenAI.

### Setup

```bash
# Using Anthropic (Claude)
CRW_EXTRACTION__LLM__PROVIDER=anthropic \
CRW_EXTRACTION__LLM__API_KEY=sk-ant-... \
./target/release/crw-server

# Using OpenAI
CRW_EXTRACTION__LLM__PROVIDER=openai \
CRW_EXTRACTION__LLM__API_KEY=sk-... \
CRW_EXTRACTION__LLM__MODEL=gpt-4o \
./target/release/crw-server
```

### Usage

Send a `jsonSchema` in the scrape request and include `"json"` in formats:

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com",
    "formats": ["json"],
    "jsonSchema": {
      "type": "object",
      "properties": {
        "title": {"type": "string"},
        "description": {"type": "string"}
      }
    }
  }'
```

The LLM reads the page markdown and returns structured data matching your schema.

---

## Logging

CRW uses the `RUST_LOG` environment variable for log level control.

```bash
# Info level (default)
RUST_LOG=info ./target/release/crw-server

# Debug level (verbose)
RUST_LOG=debug ./target/release/crw-server

# Module-specific
RUST_LOG=crw_server=debug,crw_renderer=info ./target/release/crw-server
```

---

## Security

### SSRF Protection

All URL inputs — REST API endpoints and MCP tool calls — are validated before any outbound request:

| Blocked Target | Examples |
|:---------------|:---------|
| Loopback | `127.0.0.0/8`, `::1`, `localhost` |
| Private IPs | `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16` |
| Link-local / Cloud metadata | `169.254.0.0/16` (AWS/GCP metadata endpoint) |
| IPv4-mapped IPv6 | `::ffff:127.0.0.1`, `::ffff:169.254.169.254` |
| IPv6 link-local | `fe80::/10` |
| IPv6 unique-local (ULA) | `fc00::/7` |
| Non-HTTP schemes | `file://`, `ftp://`, `gopher://`, `data:`, `blob:`, `tel:` |
| Zero/broadcast | `0.0.0.0`, `255.255.255.255` |

### Authentication

When `api_keys` is configured, **all endpoints except `/health` require a valid Bearer token** — including the `/mcp` endpoint. Token comparison uses constant-time equality that does not leak key length or key index via timing side-channels.

### robots.txt

The crawler respects robots.txt with:
- `Allow` and `Disallow` directives
- Wildcard patterns (`*` matches any character sequence, `$` anchors to end of path)
- RFC 9309 specificity-based matching (longest effective pattern wins; equal length: Allow wins)
- Inline comment stripping

### Resource Limits

| Limit | Value |
|:------|:------|
| Max request body | 1 MB |
| Max crawl depth | 10 |
| Max pages per crawl | 1,000 |
| Max discovered URLs | 5,000 |
| URL schemes allowed | `http`, `https` only |
| robots.txt | Respected by default |

### Link Extraction

Extracted links are filtered to remove dangerous URI schemes (`javascript:`, `mailto:`, `data:`, `tel:`, `blob:`) before being used in crawling or returned in API responses.
