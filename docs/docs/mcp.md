# MCP Server for AI Agents

CRW includes a built-in MCP (Model Context Protocol) server that gives any MCP-compatible AI assistant — Claude Code, Claude Desktop, Cursor, Windsurf, Cline, Continue.dev, OpenAI Codex CLI — 4 web scraping tools. Turn any AI coding agent into a web scraper with a single command.

## Two Transport Options

| Transport | Setup | Requires |
|-----------|-------|----------|
| **HTTP** (recommended) | One-liner, no binary needed | `crw-server` running |
| **Stdio** | Separate binary (`crw-mcp`) | `crw-server` running + `crw-mcp` binary |

### HTTP Transport (Recommended)

The `crw-server` has a built-in `/mcp` endpoint. No extra binary needed:

```bash
claude mcp add --transport http crw http://localhost:3000/mcp
```

### Stdio Transport

Build the standalone MCP binary:

```bash
cargo build --release --bin crw-mcp
```

The binary is at `target/release/crw-mcp` (~4 MB). It's a pure JSON-RPC 2.0 stdio proxy that forwards to the HTTP API.

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `CRW_API_URL` | `http://localhost:3000` | crw server URL |
| `CRW_API_KEY` | — | Bearer token (if auth is enabled) |
| `RUST_LOG` | `crw_mcp=info` | Log level (logs go to stderr) |

Make sure `crw-server` is running before using the MCP tools. Both transports forward requests to the HTTP API.

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

**HTTP transport (recommended):**

```bash
claude mcp add --transport http crw http://localhost:3000/mcp
```

**Stdio transport:**

```bash
claude mcp add crw -- /absolute/path/to/crw-mcp
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
      "command": "/absolute/path/to/crw-mcp",
      "env": {
        "CRW_API_URL": "http://localhost:3000"
      }
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
      "command": "/absolute/path/to/crw-mcp",
      "env": {
        "CRW_API_URL": "http://localhost:3000"
      }
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
      "command": "/absolute/path/to/crw-mcp",
      "env": {
        "CRW_API_URL": "http://localhost:3000"
      }
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

Set `alwaysAllow` for tools you trust to skip the approval prompt.

### Continue.dev (VS Code / JetBrains)

Edit `~/.continue/config.yaml`:

```yaml
mcpServers:
  - name: crw
    command: /absolute/path/to/crw-mcp
    env:
      CRW_API_URL: http://localhost:3000
```

MCP tools only work in Continue's **Agent mode**, not in regular chat.

### OpenAI Codex CLI

Edit `~/.codex/config.toml`:

```toml
[mcp_servers.crw]
command = "/absolute/path/to/crw-mcp"

[mcp_servers.crw.env]
CRW_API_URL = "http://localhost:3000"
```

Or: `codex mcp add crw -- /absolute/path/to/crw-mcp`

### Gemini CLI

Edit `~/.gemini/settings.json`:

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

### Roo Code (VS Code Extension)

Create or edit `~/.roo/mcp.json` (global) or `.roo/mcp.json` (project-level):

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

### VS Code (GitHub Copilot Agent)

Add to your VS Code `settings.json` or `.vscode/mcp.json`:

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

## Platform Comparison

| Platform | Config Format | Config Path | One-liner |
|----------|-------------|------------|-----------|
| Claude Code | JSON | `~/.claude.json` | `claude mcp add --transport http crw http://localhost:3000/mcp` |
| Claude Desktop | JSON | OS-specific | — |
| Cursor | JSON | `~/.cursor/mcp.json` | — |
| Windsurf | JSON | `~/.codeium/windsurf/mcp_config.json` | — |
| Cline | JSON | VS Code globalStorage | — |
| Continue.dev | YAML | `~/.continue/config.yaml` | — |
| OpenAI Codex | TOML | `~/.codex/config.toml` | `codex mcp add crw -- /path/to/crw-mcp` |
| Gemini CLI | JSON | `~/.gemini/settings.json` | — |
| Roo Code | JSON | `~/.roo/mcp.json` | — |
| VS Code (Copilot) | JSON | `.vscode/mcp.json` | — |

## With Authentication

If your crw server has auth enabled, add `CRW_API_KEY` to any config:

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

## Verify Installation

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
    "serverInfo": {"name": "crw-mcp", "version": "0.0.1"}
  }
}
```

## How It Works

**HTTP transport:**

```
AI Assistant → HTTP POST (JSON-RPC 2.0) → crw-server /mcp → Web pages
```

**Stdio transport:**

```
AI Assistant → stdin (JSON-RPC 2.0) → crw-mcp → HTTP → crw-server → Web pages
```

The HTTP transport calls internal functions directly with zero overhead. The stdio transport is a pure JSON proxy.

Protocol version: `2024-11-05`
