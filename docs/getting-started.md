---
title: Getting Started
layout: default
nav_order: 2
description: "Install and run CRW web scraper in under a minute. Build from source or use Docker."
---

# Getting Started
{: .no_toc }

Get CRW running in under a minute.
{: .fs-6 .fw-300 }

## Table of Contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Requirements

- **Rust 1.85+** (for building from source)
- **Docker** (optional, for containerized deployment)

## Install from crates.io

```bash
cargo install crw-server
```

For the MCP server:

```bash
cargo install crw-mcp
```

## Install with Docker (pre-built)

```bash
docker pull ghcr.io/us/crw:latest
docker run -p 3000:3000 ghcr.io/us/crw:latest
```

## Install from Source

```bash
git clone https://github.com/us/crw.git
cd crw
```

### HTTP-only (fastest build)

No JS rendering. Best for static sites and APIs.

```bash
cargo build --release --bin crw-server
```

### With JS Rendering

Adds CDP (Chrome DevTools Protocol) support for rendering SPAs.

```bash
cargo build --release --bin crw-server --features crw-server/cdp
```

### MCP Server

For Claude Code / Claude Desktop integration.

```bash
cargo build --release --bin crw-mcp
```

### Binaries Produced

| Binary | Path | Description |
|--------|------|-------------|
| `crw-server` | `target/release/crw-server` | API server |
| `crw-mcp` | `target/release/crw-mcp` | MCP server for LLM tools |

## Install with Docker Compose

```bash
git clone https://github.com/us/crw.git
cd crw
docker compose up
```

This starts:
- **crw** on port `3000` — API server with CDP enabled
- **lightpanda** on port `9222` — JS rendering sidecar

## Run the Server

```bash
./target/release/crw-server
```

You should see:

```
INFO crw_server: Starting CRW on 0.0.0.0:3000
INFO crw_server: Renderer mode: auto
INFO crw_server: CRW ready at http://0.0.0.0:3000
```

## Verify It Works

### Health Check

```bash
curl http://localhost:3000/health
```

```json
{
  "status": "ok",
  "version": "0.0.1",
  "renderers": {
    "http": true
  },
  "active_crawl_jobs": 0
}
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

## What's Next?

- [Configuration]({% link configuration.md %}) — customize ports, renderers, rate limits
- [API Reference]({% link api-reference.md %}) — all endpoints with examples
- [MCP Server]({% link mcp-server.md %}) — use CRW in Claude Code
- [Docker Deployment]({% link docker.md %}) — production setup
