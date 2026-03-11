# crw-mcp

MCP (Model Context Protocol) stdio proxy for the [CRW](https://github.com/us/crw) web scraper.

[![crates.io](https://img.shields.io/crates/v/crw-mcp.svg)](https://crates.io/crates/crw-mcp)
[![license](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](https://github.com/us/crw/blob/main/LICENSE)

## Overview

`crw-mcp` is a lightweight stdio binary that bridges MCP-compatible AI clients (Claude Code, Claude Desktop, Cursor, Windsurf, Cline, Continue.dev) to a running CRW server. It reads JSON-RPC requests from stdin, forwards them to the CRW HTTP API, and writes responses to stdout.

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

## Prerequisites

A running CRW server instance:

```bash
cargo install crw-server
crw-server
```

## Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `CRW_API_URL` | `http://localhost:3000` | CRW server base URL |
| `CRW_API_KEY` | *(none)* | Optional Bearer token for authenticated servers |

## Setup by client

### Claude Code

```bash
claude mcp add --transport stdio crw crw-mcp
```

Or use the HTTP transport directly (no binary needed):

```bash
claude mcp add --transport http crw http://localhost:3000/mcp
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
      "command": "crw-mcp",
      "env": { "CRW_API_URL": "http://localhost:3000" }
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
      "command": "crw-mcp",
      "env": { "CRW_API_URL": "http://localhost:3000" }
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
      "command": "crw-mcp",
      "env": { "CRW_API_URL": "http://localhost:3000" }
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
      "env": { "CRW_API_URL": "http://localhost:3000" },
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
    env:
      CRW_API_URL: http://localhost:3000
```

### OpenAI Codex CLI

Edit `~/.codex/config.toml`:

```toml
[mcp_servers.crw]
command = "crw-mcp"

[mcp_servers.crw.env]
CRW_API_URL = "http://localhost:3000"
```

### Any MCP client

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

> **Tip:** For clients that support HTTP transport, use `http://localhost:3000/mcp` directly — no stdio binary needed.

## With authentication

If your CRW server requires auth:

```json
{
  "mcpServers": {
    "crw": {
      "command": "crw-mcp",
      "env": {
        "CRW_API_URL": "http://localhost:3000",
        "CRW_API_KEY": "fc-your-api-key"
      }
    }
  }
}
```

## With remote / cloud server

Point to [fastcrw.com](https://fastcrw.com) or any remote CRW instance:

```json
{
  "mcpServers": {
    "crw": {
      "command": "crw-mcp",
      "env": {
        "CRW_API_URL": "https://fastcrw.com/api",
        "CRW_API_KEY": "your-cloud-api-key"
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
| **crw-mcp** | MCP stdio proxy binary (this crate) |

## License

AGPL-3.0 — see [LICENSE](https://github.com/us/crw/blob/main/LICENSE).
