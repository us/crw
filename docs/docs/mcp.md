# MCP Server for AI Agents

CRW includes a built-in MCP (Model Context Protocol) server that gives any MCP-compatible AI assistant — Claude Code, Claude Desktop, Cursor, Windsurf, Cline, Continue.dev, OpenAI Codex CLI — 4 web scraping tools. Turn any AI coding agent into a web scraper with a single command.

> Also available on the [MCP Registry](https://registry.modelcontextprotocol.io/?q=crw)

## Two Modes

`crw-mcp` supports two modes:

| Mode | When | Tools | Description |
|------|------|-------|-------------|
| **Embedded** (default) | No `--api-url` / `CRW_API_URL` set | scrape, crawl, map | Self-contained. No server needed. The scraping engine runs inside the MCP process. |
| **Proxy / Cloud** | `--api-url` / `CRW_API_URL` set | scrape, crawl, map + **search** | Forwards tool calls to a remote CRW server. Cloud mode ([fastcrw.com](https://fastcrw.com)) adds `crw_search` for web search. |

## Where to use what

- Use this page for the MCP model, tool list, and transport choices.
- Use [MCP Client Setup](#mcp-clients) for Claude Code, Codex, Cursor, Windsurf, Cline, Continue, and similar host-specific config snippets.

## When MCP Helps

MCP is useful when the agent host already expects tools to be registered through a standard interface. That reduces one layer of custom glue code between the agent and your scraping service.

Typical fits:

- agentic research workflows,
- internal copilots that need current website content,
- multi-tool assistants that combine search, scrape, and synthesis,
- and developer environments such as Claude or Cursor where MCP is already the preferred integration path.

## Quick Start (Embedded Mode)

No server to start, no setup. Install and add `crw-mcp`:

```bash
# One-line install (auto-detects OS & arch):
curl -fsSL https://raw.githubusercontent.com/us/crw/main/install.sh | sh

# npm (zero install):
npx crw-mcp

# Python (SDK — use npx or cargo for MCP binary):
pip install crw

# Cargo:
cargo install crw-mcp

# Docker:
docker run -i ghcr.io/us/crw crw-mcp
```

Add to your MCP client:

```bash
# Claude Code:
claude mcp add crw -- npx crw-mcp

# OpenAI Codex CLI:
codex mcp add crw -- npx crw-mcp
```

That's it. The agent starts `crw-mcp`, which contains the full scraping engine. When the agent disconnects, the process dies.

If you want host-by-host config files instead of one-liners, jump to [MCP Client Setup](#mcp-clients).

### With CDP rendering (LightPanda/Chrome)

If you have a CDP-compatible browser, pass it via env vars:

```bash
claude mcp add \
  -e CRW_RENDERER__LIGHTPANDA__WS_URL=ws://127.0.0.1:9222 \
  crw -- npx crw-mcp
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
  -e CRW_API_KEY=YOUR_API_KEY \
  crw -- npx crw-mcp

# Local crw-server on custom port
claude mcp add \
  -e CRW_API_URL=http://localhost:4000 \
  crw -- npx crw-mcp
```

The same `CRW_API_URL` + `CRW_API_KEY` env block also works in Codex, Claude Desktop, Cursor, Windsurf, Cline, and Continue. See [MCP Client Setup](#mcp-clients) for ready-to-paste config files.

## Three Transport Options

| Transport | Setup | Requires |
|-----------|-------|----------|
| **Stdio embedded** (recommended) | `claude mcp add crw -- npx crw-mcp` | Nothing |
| **Stdio proxy** | `CRW_API_URL=... npx crw-mcp` | Remote CRW server |
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

| Tool | Description | HTTP Endpoint | Availability |
|------|-------------|---------------|-------------|
| `crw_scrape` | Scrape a URL → markdown, HTML, links | `POST /v1/scrape` | All modes |
| `crw_crawl` | Start async crawl → returns job ID | `POST /v1/crawl` | All modes |
| `crw_check_crawl_status` | Poll crawl status and get results | `GET /v1/crawl/:id` | All modes |
| `crw_map` | Discover all URLs on a site | `POST /v1/map` | All modes |
| `crw_search` | Search the web → titles, URLs, descriptions | `POST /v1/search` | **Cloud only** |

## Best first setups

### Claude Code

```bash
# Local embedded
claude mcp add crw -- npx crw-mcp

# fastcrw.com cloud
claude mcp add \
  -e CRW_API_URL=https://fastcrw.com/api \
  -e CRW_API_KEY=YOUR_API_KEY \
  crw -- npx crw-mcp
```

### OpenAI Codex CLI

```bash
# Local embedded
codex mcp add crw -- npx crw-mcp
```

For cloud mode and file-based configs, continue in [MCP Client Setup](#mcp-clients).

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

### crw_search (cloud only)

Available only when connected to [fastcrw.com](https://fastcrw.com) via `CRW_API_URL`. Not available in embedded or self-hosted mode.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `query` | string | **yes** | The search query |
| `limit` | integer | no | Max results (default: 5) |
| `lang` | string | no | Language code (e.g. `"en"`, `"tr"`) |
| `country` | string | no | Country code (e.g. `"us"`, `"tr"`) |
| `scrapeOptions` | object | no | Scrape each result page (e.g. `{"formats": ["markdown"]}`) |

## Example Agent Tool Flow

A clean MCP setup often assigns each CRW route a narrow purpose:

- `search` for web discovery when you don't know the URL (cloud only),
- `map` for site-specific URL discovery,
- `scrape` for single-page extraction,
- `crawl` for bounded recursive work.

That keeps tool selection obvious for the host agent. If you expose one broad "web tool" instead, agents tend to overuse it and produce noisier traces.

A common workflow:

1. The agent identifies a site or page it needs.
2. It calls an MCP-exposed CRW tool.
3. CRW returns scrape, map, or crawl output.
4. The agent decides whether to continue exploring or move into summarization, ranking, or retrieval.

## When MCP Is Better Than Direct HTTP

Choose MCP when the host environment already expects tool discovery through a shared protocol, especially in local agent runtimes or IDE workflows. Choose direct HTTP when your application already owns orchestration and just needs API access from the backend.

In other words, MCP is ideal when the caller is an agent platform. Direct HTTP is often simpler when the caller is your own service code.

## Detailed host setup

Use [MCP Client Setup](#mcp-clients) for:

- Claude Code one-liners and HTTP transport
- Codex CLI one-liners and `~/.codex/config.toml`
- Claude Desktop config paths
- Cursor, Windsurf, and Cline JSON examples
- Continue YAML examples
- local embedded and fastcrw.com cloud config variants side by side

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
    "serverInfo": {"name": "crw-mcp", "version": "<current>"}
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

## Operational Notes

- Keep MCP tool descriptions tight so the agent knows when to use `map` versus `scrape`.
- Start with read-only scraping tools before exposing anything more complex in the same MCP server.
- Log tool usage separately from downstream agent reasoning so debugging stays tractable.

## Common Mistakes

- Registering ambiguous tool descriptions that do not explain when to use `map` versus `scrape`.
- Mixing operational secrets and agent prompts in the same configuration surface.
- Assuming MCP replaces deployment or auth decisions; it only standardizes the tool interface.
