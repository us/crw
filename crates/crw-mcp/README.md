# crw-mcp

MCP (Model Context Protocol) server for the [CRW](https://github.com/us/crw) web scraper.

[![crates.io](https://img.shields.io/crates/v/crw-mcp.svg)](https://crates.io/crates/crw-mcp)
[![license](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](https://github.com/us/crw/blob/main/LICENSE)

## Overview

`crw-mcp` is a self-contained MCP server that gives any MCP-compatible AI client (Claude Code, Claude Desktop, Cursor, Windsurf, Cline, Continue.dev, OpenAI Codex CLI) 4 web scraping tools. No external server needed — just add and go.

**Two modes:**

| Mode | When | Setup |
|------|------|-------|
| **Embedded** (default) | No `--api-url` set | Self-contained, zero setup |
| **Proxy** | `--api-url` provided | Forwards to remote CRW server |

**4 MCP tools:**

| Tool | Description |
|------|-------------|
| `crw_scrape` | Scrape a single URL → markdown, HTML, JSON, links |
| `crw_crawl` | Start an async BFS crawl (returns job ID) |
| `crw_check_crawl_status` | Poll crawl job status and retrieve results |
| `crw_map` | Discover all URLs on a website |

## Installation

```bash
cargo install crw-mcp
```

## Quick Start (Embedded Mode)

No server to run. Just add `crw-mcp` to your AI client:

```bash
# Claude Code
claude mcp add crw -- crw-mcp

# With custom config via env vars
claude mcp add \
  -e CRW_CRAWLER__MAX_CONCURRENCY=5 \
  -e CRW_RENDERER__LIGHTPANDA__WS_URL=ws://127.0.0.1:9222 \
  crw -- crw-mcp
```

## Proxy Mode (Remote Server)

Connect to [fastcrw.com](https://fastcrw.com) or any remote CRW instance:

```bash
# Cloud server
claude mcp add \
  -e CRW_API_URL=https://fastcrw.com/api \
  -e CRW_API_KEY=fc-xxx \
  crw -- crw-mcp

# Local crw-server on custom port
claude mcp add \
  -e CRW_API_URL=http://localhost:4000 \
  crw -- crw-mcp
```

Or use the HTTP transport directly (no `crw-mcp` binary needed):

```bash
claude mcp add --transport http crw http://localhost:3000/mcp
```

## CLI Options

| Flag | Env Var | Description |
|------|---------|-------------|
| `--api-url` | `CRW_API_URL` | Remote server URL (enables proxy mode) |
| `--api-key` | `CRW_API_KEY` | Bearer token for remote server auth |
| `--config` | `CRW_CONFIG` | Config file path (embedded mode only) |
| — | `RUST_LOG` | Log level (default: `crw_mcp=info`, logs go to stderr) |

### Embedded mode configuration

In embedded mode, `crw-mcp` loads configuration the same way as `crw-server`: `config.default.toml` → `config.local.toml` → environment variable overrides. Env vars use `CRW_` prefix with `__` as separator:

```bash
CRW_CRAWLER__MAX_CONCURRENCY=5
CRW_RENDERER__LIGHTPANDA__WS_URL=ws://127.0.0.1:9222
CRW_CRAWLER__USER_AGENT="MyBot/1.0"
```

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `embedded` | on | Self-contained scraping engine (pulls in `crw-server`) |

Build a slim proxy-only binary without the embedded engine:

```bash
cargo build -p crw-mcp --no-default-features --release
```

## Setup by Client

### Claude Code

```bash
claude mcp add crw -- crw-mcp
```

### Claude Desktop

Edit your config file:

| OS | Path |
|---|---|
| macOS | `~/Library/Application Support/Claude/claude_desktop_config.json` |
| Windows | `%APPDATA%\Claude\claude_desktop_config.json` |
| Linux | `~/.config/Claude/claude_desktop_config.json` |

```json
{
  "mcpServers": {
    "crw": {
      "command": "crw-mcp"
    }
  }
}
```

### Cursor

Edit `~/.cursor/mcp.json` (global) or `.cursor/mcp.json` (project):

```json
{
  "mcpServers": {
    "crw": {
      "command": "crw-mcp"
    }
  }
}
```

### Windsurf

Edit `~/.codeium/windsurf/mcp_config.json`:

```json
{
  "mcpServers": {
    "crw": {
      "command": "crw-mcp"
    }
  }
}
```

### Cline (VS Code)

```json
{
  "mcpServers": {
    "crw": {
      "command": "crw-mcp",
      "alwaysAllow": ["crw_scrape", "crw_map"],
      "disabled": false
    }
  }
}
```

### Continue.dev (VS Code / JetBrains)

Edit `~/.continue/config.yaml`:

```yaml
mcpServers:
  - name: crw
    command: crw-mcp
```

### OpenAI Codex CLI

Edit `~/.codex/config.toml`:

```toml
[mcp_servers.crw]
command = "crw-mcp"
```

### Any MCP client

```json
{
  "mcpServers": {
    "crw": {
      "command": "crw-mcp"
    }
  }
}
```

> **Tip:** For clients that support HTTP transport, you can still use `http://localhost:3000/mcp` directly with a running `crw-server` — no stdio binary needed.

## With Proxy Mode Authentication

```json
{
  "mcpServers": {
    "crw": {
      "command": "crw-mcp",
      "env": {
        "CRW_API_URL": "https://fastcrw.com/api",
        "CRW_API_KEY": "fc-your-api-key"
      }
    }
  }
}
```

## Part of CRW

This crate is part of the [CRW](https://github.com/us/crw) workspace — a fast, lightweight, Firecrawl-compatible web scraper built in Rust.

| Crate | Description |
|-------|-------------|
| [crw-core](https://crates.io/crates/crw-core) | Core types, config, and error handling |
| [crw-renderer](https://crates.io/crates/crw-renderer) | HTTP + CDP browser rendering engine |
| [crw-extract](https://crates.io/crates/crw-extract) | HTML → markdown/plaintext extraction |
| [crw-crawl](https://crates.io/crates/crw-crawl) | Async BFS crawler with robots.txt & sitemap |
| [crw-server](https://crates.io/crates/crw-server) | Firecrawl-compatible API server |
| [crw-cli](https://crates.io/crates/crw-cli) | Standalone CLI (`crw` binary) |
| **crw-mcp** | MCP server binary (this crate) |

## License

AGPL-3.0 — see [LICENSE](https://github.com/us/crw/blob/main/LICENSE).
