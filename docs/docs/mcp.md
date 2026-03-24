# MCP Server for AI Agents

CRW includes a built-in MCP (Model Context Protocol) server that gives any MCP-compatible AI assistant — Claude Code, Claude Desktop, Cursor, Windsurf, Cline, Continue.dev, OpenAI Codex CLI — 4 web scraping tools. Turn any AI coding agent into a web scraper with a single command.

## Two Modes

`crw-mcp` supports two modes:

| Mode | When | Description |
|------|------|-------------|
| **Embedded** (default) | No `--api-url` / `CRW_API_URL` set | Self-contained. No server needed. The scraping engine runs inside the MCP process. |
| **Proxy** | `--api-url` / `CRW_API_URL` set | Forwards tool calls to a remote CRW server (fastcrw.com, self-hosted, etc.) |

## Quick Start (Embedded Mode)

No server to start, no setup. Just add `crw-mcp`:

```bash
claude mcp add crw -- crw-mcp
```

That's it. The agent starts `crw-mcp`, which contains the full scraping engine. When the agent disconnects, the process dies.

### With CDP rendering (LightPanda/Chrome)

If you have a CDP-compatible browser, pass it via env vars:

```bash
claude mcp add \
  -e CRW_RENDERER__LIGHTPANDA__WS_URL=ws://127.0.0.1:9222 \
  crw -- crw-mcp
```

Without a CDP browser, `crw-mcp` uses its HTTP-only renderer (no JavaScript rendering).

### Embedded mode configuration

In embedded mode, `crw-mcp` loads config the same way as `crw-server`: `config.default.toml` → `config.local.toml` → env var overrides. Env vars use `CRW_` prefix with `__` separator:

```bash
CRW_CRAWLER__MAX_CONCURRENCY=5
CRW_RENDERER__LIGHTPANDA__WS_URL=ws://127.0.0.1:9222
CRW_CRAWLER__USER_AGENT="MyBot/1.0"
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

## Three Transport Options

| Transport | Setup | Requires |
|-----------|-------|----------|
| **Stdio embedded** (recommended) | `claude mcp add crw -- crw-mcp` | Nothing |
| **Stdio proxy** | `CRW_API_URL=... crw-mcp` | Remote CRW server |
| **HTTP** | `claude mcp add --transport http crw http://localhost:3000/mcp` | `crw-server` running |

### HTTP Transport

The `crw-server` has a built-in `/mcp` endpoint. No extra binary needed:

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

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `embedded` | on | Self-contained scraping engine (pulls in `crw-server`) |

Build a slim proxy-only binary:

```bash
cargo build -p crw-mcp --no-default-features --release
```

## Available Tools

| Tool | Description | HTTP Endpoint |
|------|-------------|---------------|
| `crw_scrape` | Scrape a URL → markdown, HTML, links | `POST /v1/scrape` |
| `crw_crawl` | Start async crawl → returns job ID | `POST /v1/crawl` |
| `crw_check_crawl_status` | Poll crawl status and get results | `GET /v1/crawl/:id` |
| `crw_map` | Discover all URLs on a site | `POST /v1/map` |

### crw_scrape

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `url` | string | **yes** | The URL to scrape |
| `formats` | string[] | no | `markdown`, `html`, `links` |
| `onlyMainContent` | boolean | no | Strip nav/footer (default: true) |
| `includeTags` | string[] | no | CSS selectors to keep |
| `excludeTags` | string[] | no | CSS selectors to remove |

### crw_crawl

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `url` | string | **yes** | Starting URL |
| `maxDepth` | integer | no | Max crawl depth (default: 2) |
| `maxPages` | integer | no | Max pages (default: 10) |

### crw_check_crawl_status

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `id` | string | **yes** | Job ID from `crw_crawl` |

### crw_map

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `url` | string | **yes** | URL to map |
| `maxDepth` | integer | no | Discovery depth (default: 2) |
| `useSitemap` | boolean | no | Read sitemap.xml (default: true) |

## Platform Setup Guides

### Claude Code

```bash
# Embedded mode (recommended — no server needed)
claude mcp add crw -- crw-mcp

# Proxy mode (remote server)
claude mcp add -e CRW_API_URL=https://fastcrw.com/api -e CRW_API_KEY=fc-xxx crw -- crw-mcp

# HTTP transport (requires crw-server running)
claude mcp add --transport http crw http://localhost:3000/mcp
```

Use `claude mcp list` to verify, `claude mcp remove crw` to uninstall.

### Claude Desktop

Edit the config file for your OS:

| OS | Path |
|----|------|
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

Fully quit and restart Claude Desktop.

### Cursor

Create or edit `~/.cursor/mcp.json` (global) or `.cursor/mcp.json` (project-level):

```json
{
  "mcpServers": {
    "crw": {
      "command": "crw-mcp"
    }
  }
}
```

Or use the GUI: **Settings → Developer → MCP Tools → Add Custom MCP**.

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

Windsurf has a limit of 100 total tools across all MCP servers. crw uses only 4.

### Cline (VS Code Extension)

| OS | Path |
|----|------|
| macOS | `~/Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json` |
| Windows | `%APPDATA%/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json` |
| Linux | `~/.config/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json` |

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

Set `alwaysAllow` for tools you trust to skip the approval prompt.

### Continue.dev (VS Code / JetBrains)

Edit `~/.continue/config.yaml`:

```yaml
mcpServers:
  - name: crw
    command: crw-mcp
```

MCP tools only work in Continue's **Agent mode**, not in regular chat.

### OpenAI Codex CLI

Edit `~/.codex/config.toml`:

```toml
[mcp_servers.crw]
command = "crw-mcp"
```

Or: `codex mcp add crw -- crw-mcp`

### Gemini CLI

Edit `~/.gemini/settings.json`:

```json
{
  "mcpServers": {
    "crw": {
      "command": "crw-mcp"
    }
  }
}
```

### Roo Code (VS Code Extension)

Create or edit `~/.roo/mcp.json` (global) or `.roo/mcp.json` (project-level):

```json
{
  "mcpServers": {
    "crw": {
      "command": "crw-mcp"
    }
  }
}
```

### VS Code (GitHub Copilot Agent)

Add to your VS Code `settings.json` or `.vscode/mcp.json`:

```json
{
  "mcpServers": {
    "crw": {
      "command": "crw-mcp"
    }
  }
}
```

## Platform Comparison

| Platform | Config Format | Config Path | One-liner |
|----------|-------------|------------|-----------|
| Claude Code | JSON | `~/.claude.json` | `claude mcp add crw -- crw-mcp` |
| Claude Desktop | JSON | OS-specific | — |
| Cursor | JSON | `~/.cursor/mcp.json` | — |
| Windsurf | JSON | `~/.codeium/windsurf/mcp_config.json` | — |
| Cline | JSON | VS Code globalStorage | — |
| Continue.dev | YAML | `~/.continue/config.yaml` | — |
| OpenAI Codex | TOML | `~/.codex/config.toml` | `codex mcp add crw -- crw-mcp` |
| Gemini CLI | JSON | `~/.gemini/settings.json` | — |
| Roo Code | JSON | `~/.roo/mcp.json` | — |
| VS Code (Copilot) | JSON | `.vscode/mcp.json` | — |

## With Proxy Mode Authentication

If connecting to a remote server with auth, add `CRW_API_URL` and `CRW_API_KEY`:

```json
{
  "mcpServers": {
    "crw": {
      "command": "crw-mcp",
      "env": {
        "CRW_API_URL": "https://fastcrw.com/api",
        "CRW_API_KEY": "your-api-key"
      }
    }
  }
}
```

## Verify Installation

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"test"},"protocolVersion":"2024-11-05"}}' \
  | crw-mcp 2>/dev/null
```

Expected:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "protocolVersion": "2024-11-05",
    "capabilities": {"tools": {}},
    "serverInfo": {"name": "crw-mcp", "version": "0.0.12"}
  }
}
```

## How It Works

**Embedded mode (default):**

```
AI Assistant → stdin (JSON-RPC 2.0) → crw-mcp [scraping engine] → Web pages
```

**Proxy mode:**

```
AI Assistant → stdin (JSON-RPC 2.0) → crw-mcp → HTTP → crw-server → Web pages
```

**HTTP transport:**

```
AI Assistant → HTTP POST (JSON-RPC 2.0) → crw-server /mcp → Web pages
```

In embedded mode, the scraping engine runs in-process with zero overhead. In proxy mode, tool calls are forwarded over HTTP. The HTTP transport calls `crw-server` functions directly.

Protocol version: `2024-11-05`
