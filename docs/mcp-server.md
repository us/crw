---
title: MCP Server
layout: default
nav_order: 5
description: "Use CRW as a tool in Claude Code and Claude Desktop via MCP (Model Context Protocol). Setup guide and tool reference."
---

# MCP Server
{: .no_toc }

Use CRW as a web scraping tool directly in Claude Code and Claude Desktop.
{: .fs-6 .fw-300 }

## Table of Contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## What is MCP?

[MCP (Model Context Protocol)](https://modelcontextprotocol.io) lets AI assistants like Claude use external tools. CRW's MCP server is a lightweight stdio proxy — it reads JSON-RPC 2.0 from stdin, calls the CRW HTTP API, and writes results to stdout.

## Build

```bash
cargo build --release --bin crw-mcp
```

The binary is at `target/release/crw-mcp` (~4 MB).

## Setup for Claude Code

Add to `~/.claude.json`:

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
        "CRW_API_KEY": "your-api-key"
      }
    }
  }
}
```

Restart Claude Code after saving the config.

## Setup for Claude Desktop

Add the same config to:

| OS | Path |
|:---|:-----|
| macOS | `~/Library/Application Support/Claude/claude_desktop_config.json` |
| Windows | `%APPDATA%\Claude\claude_desktop_config.json` |

Restart Claude Desktop after saving.

## Environment Variables

| Variable | Default | Description |
|:---------|:--------|:------------|
| `CRW_API_URL` | `http://localhost:3000` | CRW server URL |
| `CRW_API_KEY` | — | Bearer token (if auth is enabled on server) |
| `RUST_LOG` | `crw_mcp=info` | Log level (logs go to stderr) |

## Available Tools

After setup, these tools appear in Claude:

### crw_scrape

Scrape a single URL and return its content.

| Parameter | Type | Required | Description |
|:----------|:-----|:---------|:------------|
| `url` | string | **yes** | The URL to scrape |
| `formats` | string[] | no | `markdown`, `html`, `links` |
| `onlyMainContent` | boolean | no | Strip nav/footer (default: true) |
| `includeTags` | string[] | no | CSS selectors to keep |
| `excludeTags` | string[] | no | CSS selectors to remove |

### crw_crawl

Start an async crawl job. Returns a job ID.

| Parameter | Type | Required | Description |
|:----------|:-----|:---------|:------------|
| `url` | string | **yes** | Starting URL |
| `maxDepth` | integer | no | Max crawl depth (default: 2) |
| `maxPages` | integer | no | Max pages (default: 10) |

### crw_check_crawl_status

Poll a crawl job for status and results.

| Parameter | Type | Required | Description |
|:----------|:-----|:---------|:------------|
| `id` | string | **yes** | Job ID from `crw_crawl` |

### crw_map

Discover all URLs on a website.

| Parameter | Type | Required | Description |
|:----------|:-----|:---------|:------------|
| `url` | string | **yes** | URL to map |
| `maxDepth` | integer | no | Discovery depth (default: 2) |
| `useSitemap` | boolean | no | Read sitemap.xml (default: true) |

## Verify Installation

Make sure `crw-server` is running, then test the MCP server:

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"test"},"protocolVersion":"2024-11-05"}}' \
  | ./target/release/crw-mcp 2>/dev/null
```

Expected output:

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
Claude Code/Desktop
    ↓ stdin (JSON-RPC 2.0)
  crw-mcp
    ↓ HTTP (POST/GET)
  crw-server (localhost:3000)
    ↓
  Web pages
```

The MCP server has **zero dependency** on CRW's internal crates. It's a pure JSON proxy — any Firecrawl-compatible API works as a backend.
