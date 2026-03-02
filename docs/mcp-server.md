---
title: MCP Server
layout: default
nav_order: 5
description: "Use CRW as a web scraping tool in Claude Code, Claude Desktop, Cursor, Windsurf, Cline, Continue.dev, and OpenAI Codex via MCP."
has_children: true
---

# MCP Server
{: .no_toc }

Use CRW as a web scraping tool in any AI coding assistant that supports MCP.
{: .fs-6 .fw-300 }

## Table of Contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## What is MCP?

[MCP (Model Context Protocol)](https://modelcontextprotocol.io) is an open standard that lets AI assistants use external tools. CRW ships with a built-in MCP server that gives any MCP-compatible client 4 web scraping tools.

## Build the MCP Server

```bash
cargo build --release --bin crw-mcp
```

The binary is at `target/release/crw-mcp` (~4 MB). It has **zero dependency** on CRW's internal crates — it's a pure JSON-RPC 2.0 stdio proxy.

{: .warning }
Make sure `crw-server` is running before using the MCP tools. The MCP server forwards all requests to the HTTP API.

## Available Tools

Once connected, these tools appear in your AI assistant:

| Tool | Description | HTTP Endpoint |
|:-----|:------------|:-------------|
| `crw_scrape` | Scrape a URL → markdown, HTML, links | `POST /v1/scrape` |
| `crw_crawl` | Start async crawl → returns job ID | `POST /v1/crawl` |
| `crw_check_crawl_status` | Poll crawl status and get results | `GET /v1/crawl/:id` |
| `crw_map` | Discover all URLs on a site | `POST /v1/map` |

### Tool Parameters

**crw_scrape:**

| Parameter | Type | Required | Description |
|:----------|:-----|:---------|:------------|
| `url` | string | **yes** | The URL to scrape |
| `formats` | string[] | no | `markdown`, `html`, `links` |
| `onlyMainContent` | boolean | no | Strip nav/footer (default: true) |
| `includeTags` | string[] | no | CSS selectors to keep |
| `excludeTags` | string[] | no | CSS selectors to remove |

**crw_crawl:**

| Parameter | Type | Required | Description |
|:----------|:-----|:---------|:------------|
| `url` | string | **yes** | Starting URL |
| `maxDepth` | integer | no | Max crawl depth (default: 2) |
| `maxPages` | integer | no | Max pages (default: 10) |

**crw_check_crawl_status:**

| Parameter | Type | Required | Description |
|:----------|:-----|:---------|:------------|
| `id` | string | **yes** | Job ID from `crw_crawl` |

**crw_map:**

| Parameter | Type | Required | Description |
|:----------|:-----|:---------|:------------|
| `url` | string | **yes** | URL to map |
| `maxDepth` | integer | no | Discovery depth (default: 2) |
| `useSitemap` | boolean | no | Read sitemap.xml (default: true) |

## MCP Environment Variables

| Variable | Default | Description |
|:---------|:--------|:------------|
| `CRW_API_URL` | `http://localhost:3000` | CRW server URL |
| `CRW_API_KEY` | — | Bearer token (if auth is enabled) |
| `RUST_LOG` | `crw_mcp=info` | Log level (logs go to stderr) |

---

## Platform Setup Guides

### Claude Code

One-liner:

```bash
claude mcp add crw -- /absolute/path/to/crw-mcp
```

With environment variables:

```bash
claude mcp add --env CRW_API_URL=http://localhost:3000 crw -- /absolute/path/to/crw-mcp
```

Or manually edit `~/.claude.json`:

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

For project-level config, create `.mcp.json` in your project root with the same format.

Restart Claude Code after adding.

{: .tip }
Use `claude mcp list` to verify CRW is registered, and `claude mcp remove crw` to uninstall.

---

### Claude Desktop

Edit the config file for your OS:

| OS | Path |
|:---|:-----|
| macOS | `~/Library/Application Support/Claude/claude_desktop_config.json` |
| Windows | `%APPDATA%\Claude\claude_desktop_config.json` |
| Linux | `~/.config/Claude/claude_desktop_config.json` |

Add:

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

Fully quit and restart Claude Desktop.

---

### Cursor

Create or edit `~/.cursor/mcp.json` (global) or `.cursor/mcp.json` (project-level):

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

Or use the GUI: **Settings → Developer → MCP Tools → Add Custom MCP**.

Cursor supports stdio and streamable HTTP transports.

---

### Windsurf

Edit `~/.codeium/windsurf/mcp_config.json`:

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

Or use the GUI: **Windsurf Settings → Cascade → MCP Servers**.

{: .note }
Windsurf has a limit of 100 total tools across all MCP servers. CRW uses only 4.

---

### Cline (VS Code Extension)

Config file location depends on your OS:

| OS | Path |
|:---|:-----|
| macOS | `~/Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json` |
| Windows | `%APPDATA%/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json` |
| Linux | `~/.config/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json` |

```json
{
  "mcpServers": {
    "crw": {
      "command": "/absolute/path/to/crw-mcp",
      "env": {
        "CRW_API_URL": "http://localhost:3000"
      },
      "alwaysAllow": ["crw_scrape", "crw_map"],
      "disabled": false
    }
  }
}
```

Or use the GUI: click the **MCP Servers** icon in Cline's top bar → Configure → "Configure MCP Servers".

{: .tip }
Set `alwaysAllow` for tools you trust to skip the approval prompt every time.

---

### Continue.dev (VS Code / JetBrains)

Edit `~/.continue/config.yaml`:

```yaml
mcpServers:
  - name: crw
    command: /absolute/path/to/crw-mcp
    env:
      CRW_API_URL: http://localhost:3000
```

Or drop a JSON file at `.continue/mcpServers/crw.json` in your project:

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

{: .note }
MCP tools only work in Continue's **Agent mode**, not in regular chat.

---

### OpenAI Codex CLI

Edit `~/.codex/config.toml`:

```toml
[mcp_servers.crw]
command = "/absolute/path/to/crw-mcp"

[mcp_servers.crw.env]
CRW_API_URL = "http://localhost:3000"
```

Or use the CLI:

```bash
codex mcp add crw -- /absolute/path/to/crw-mcp
```

---

## Platform Comparison

| Platform | Config Format | Config Path | One-liner |
|:---------|:-------------|:------------|:----------|
| Claude Code | JSON | `~/.claude.json` | `claude mcp add crw -- /path/to/crw-mcp` |
| Claude Desktop | JSON | OS-specific (see above) | — |
| Cursor | JSON | `~/.cursor/mcp.json` | — |
| Windsurf | JSON | `~/.codeium/windsurf/mcp_config.json` | — |
| Cline | JSON | VS Code globalStorage | — |
| Continue.dev | YAML | `~/.continue/config.yaml` | — |
| OpenAI Codex | TOML | `~/.codex/config.toml` | `codex mcp add crw -- /path/to/crw-mcp` |

---

## With Authentication

If your CRW server has auth enabled, add the `CRW_API_KEY` env var to any config above:

```json
{
  "mcpServers": {
    "crw": {
      "command": "/absolute/path/to/crw-mcp",
      "env": {
        "CRW_API_URL": "http://localhost:3000",
        "CRW_API_KEY": "your-api-key"
      }
    }
  }
}
```

---

## Verify Installation

Test the MCP server directly:

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

## How It Works

```
AI Assistant (Claude, Cursor, Codex, ...)
    ↓ stdin (JSON-RPC 2.0)
  crw-mcp
    ↓ HTTP (POST/GET)
  crw-server (localhost:3000)
    ↓
  Web pages
```

The MCP server is a pure JSON proxy. It works with any Firecrawl-compatible API backend — not just CRW.
