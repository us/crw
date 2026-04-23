<div class="page-intro">
  <div class="page-kicker">Integrations</div>
  <h1>MCP Client Setup</h1>
  <p class="page-subtitle">Add CRW to Claude Code, Codex, Cursor, Windsurf, Cline, Continue, and similar MCP hosts. This page is the copy-paste setup reference for both local embedded mode and fastcrw.com cloud mode.</p>
  <div class="page-capabilities">
    <div class="page-capability"><strong>Best for:</strong> host-by-host setup</div>
    <div class="page-capability"><strong>Local mode:</strong> <code>npx crw-mcp</code></div>
    <div class="page-capability"><strong>Cloud mode:</strong> <code>CRW_API_URL + CRW_API_KEY</code></div>
  </div>
  <div class="page-actions">
    <a class="page-btn primary" href="#mcp">Open MCP Overview</a>
    <a class="page-btn secondary" href="https://fastcrw.com" target="_blank" rel="noopener">Get fastcrw.com API Key</a>
  </div>
</div>

## Start here

Use one of these three patterns:

| Pattern | Best when | Copy-paste shape |
|---|---|---|
| Embedded local | You want zero server setup and local scrape/map/crawl tools | `command: npx`, `args: ["crw-mcp"]` |
| fastcrw.com cloud | You want hosted infrastructure and `crw_search` | same as local plus `CRW_API_URL` and `CRW_API_KEY` |
| HTTP transport | Your host supports HTTP MCP and you already run `crw-server` | point the client at `http://localhost:3000/mcp` |
| Browser automation | You need a real, stateful browser (click, nav, tree) | `command: crw-browse` (separate MCP server — self-hosted) |

## What tools you get

Embedded local mode (`crw-mcp`) exposes:

- `crw_scrape`
- `crw_crawl`
- `crw_check_crawl_status`
- `crw_map`

fastcrw.com cloud mode exposes all of the above plus:

- `crw_search`

Browser automation mode (`crw-browse`, separate server — v0.4.0+) exposes:

- `goto` — navigate the browser to an URL
- `tree` — accessibility snapshot of the current page

:::tip
If you only remember one rule, remember this one: local embedded mode is the easiest setup, and fastcrw.com cloud mode is the easiest way to add web search.
:::

## Claude Code

### Local embedded

```bash
claude mcp add crw -- npx crw-mcp
```

### fastcrw.com cloud

```bash
claude mcp add \
  -e CRW_API_URL=https://fastcrw.com/api \
  -e CRW_API_KEY=YOUR_API_KEY \
  crw -- npx crw-mcp
```

### Local HTTP transport

```bash
claude mcp add --transport http crw http://localhost:3000/mcp
```

Useful commands:

- `claude mcp list`
- `claude mcp remove crw`

## OpenAI Codex CLI

### Local embedded

```bash
codex mcp add crw -- npx crw-mcp
```

Or edit `~/.codex/config.toml`:

```toml
[mcp_servers.crw]
command = "npx"
args = ["crw-mcp"]
```

### fastcrw.com cloud

```toml
[mcp_servers.crw]
command = "npx"
args = ["crw-mcp"]

[mcp_servers.crw.env]
CRW_API_URL = "https://fastcrw.com/api"
CRW_API_KEY = "YOUR_API_KEY"
```

:::note
Codex uses the same stdio MCP server as Claude Code. The only difference is where you register it.
:::

## Claude Desktop

Edit the config file for your OS:

| OS | Path |
|---|---|
| macOS | `~/Library/Application Support/Claude/claude_desktop_config.json` |
| Windows | `%APPDATA%/Claude/claude_desktop_config.json` |
| Linux | `~/.config/Claude/claude_desktop_config.json` |

### Local embedded

```json
{
  "mcpServers": {
    "crw": {
      "command": "npx",
      "args": ["crw-mcp"]
    }
  }
}
```

### fastcrw.com cloud

```json
{
  "mcpServers": {
    "crw": {
      "command": "npx",
      "args": ["crw-mcp"],
      "env": {
        "CRW_API_URL": "https://fastcrw.com/api",
        "CRW_API_KEY": "YOUR_API_KEY"
      }
    }
  }
}
```

Fully quit and restart Claude Desktop after editing the file.

## Cursor

Create or edit `~/.cursor/mcp.json` for global setup, or `.cursor/mcp.json` inside one project.

### Local embedded

```json
{
  "mcpServers": {
    "crw": {
      "command": "npx",
      "args": ["crw-mcp"]
    }
  }
}
```

### fastcrw.com cloud

```json
{
  "mcpServers": {
    "crw": {
      "command": "npx",
      "args": ["crw-mcp"],
      "env": {
        "CRW_API_URL": "https://fastcrw.com/api",
        "CRW_API_KEY": "YOUR_API_KEY"
      }
    }
  }
}
```

You can also add the same config from Cursor's MCP UI under Settings.

## Windsurf

Edit `~/.codeium/windsurf/mcp_config.json`.

### Local embedded

```json
{
  "mcpServers": {
    "crw": {
      "command": "npx",
      "args": ["crw-mcp"]
    }
  }
}
```

### fastcrw.com cloud

```json
{
  "mcpServers": {
    "crw": {
      "command": "npx",
      "args": ["crw-mcp"],
      "env": {
        "CRW_API_URL": "https://fastcrw.com/api",
        "CRW_API_KEY": "YOUR_API_KEY"
      }
    }
  }
}
```

:::note
Windsurf has a total MCP tool limit. CRW stays lightweight: local mode exposes 4 tools, cloud mode exposes 5.
:::

## Cline

Edit the Cline MCP settings file:

| OS | Path |
|---|---|
| macOS | `~/Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json` |
| Windows | `%APPDATA%/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json` |
| Linux | `~/.config/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json` |

### Local embedded

```json
{
  "mcpServers": {
    "crw": {
      "command": "npx",
      "args": ["crw-mcp"],
      "alwaysAllow": ["crw_scrape", "crw_map"],
      "disabled": false
    }
  }
}
```

### fastcrw.com cloud

```json
{
  "mcpServers": {
    "crw": {
      "command": "npx",
      "args": ["crw-mcp"],
      "alwaysAllow": ["crw_scrape", "crw_map", "crw_search"],
      "disabled": false,
      "env": {
        "CRW_API_URL": "https://fastcrw.com/api",
        "CRW_API_KEY": "YOUR_API_KEY"
      }
    }
  }
}
```

## Continue

Edit `~/.continue/config.yaml`.

### Local embedded

```yaml
mcpServers:
  - name: crw
    command: npx
    args:
      - crw-mcp
```

### fastcrw.com cloud

```yaml
mcpServers:
  - name: crw
    command: npx
    args:
      - crw-mcp
    env:
      CRW_API_URL: https://fastcrw.com/api
      CRW_API_KEY: YOUR_API_KEY
```

MCP tools in Continue only work in Agent mode, not plain chat mode.

## Gemini CLI

### Local embedded

```bash
gemini mcp add crw -- npx crw-mcp
```

Or edit `~/.gemini/settings.json`:

```json
{
  "mcpServers": {
    "crw": {
      "command": "npx",
      "args": ["crw-mcp"]
    }
  }
}
```

### fastcrw.com cloud

```json
{
  "mcpServers": {
    "crw": {
      "command": "npx",
      "args": ["crw-mcp"],
      "env": {
        "CRW_API_URL": "https://fastcrw.com/api",
        "CRW_API_KEY": "YOUR_API_KEY"
      }
    }
  }
}
```

Useful commands:

- `gemini mcp list`
- `gemini mcp remove crw`

## Any MCP client

If your client accepts the standard JSON `mcpServers` shape, start here:

### Local embedded

```json
{
  "mcpServers": {
    "crw": {
      "command": "npx",
      "args": ["crw-mcp"]
    }
  }
}
```

### fastcrw.com cloud

```json
{
  "mcpServers": {
    "crw": {
      "command": "npx",
      "args": ["crw-mcp"],
      "env": {
        "CRW_API_URL": "https://fastcrw.com/api",
        "CRW_API_KEY": "YOUR_API_KEY"
      }
    }
  }
}
```

## Local rendering and config

Local embedded mode uses the same config chain as `crw-server`:

- `config.default.toml`
- `config.local.toml`
- `CRW_...` environment overrides

If you need JavaScript rendering in local MCP mode, pass a renderer URL:

```bash
claude mcp add \
  -e CRW_RENDERER__LIGHTPANDA__WS_URL=ws://127.0.0.1:9222 \
  crw -- npx crw-mcp
```

Without a configured renderer, CRW still works in HTTP-only mode.

## How to choose local vs fastcrw.com

Use local embedded mode when:

- you want zero infrastructure,
- scrape/map/crawl are enough,
- or you want everything to stay on your own machine.

Use fastcrw.com when:

- you want managed infrastructure,
- you want the `crw_search` tool,
- or you do not want to manage a local renderer or server.

## What to read next

- [MCP Server](#mcp) for the mode overview
- [Search](#search) to see what cloud mode adds
- [JS Rendering](#js-rendering) if local embedded mode needs browser rendering
- [Authentication](#authentication) if you are using a self-hosted CRW server
