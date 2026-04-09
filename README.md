<p align="center">
  <a href="https://fastcrw.com">
    <img src="docs/logo.png" alt="fastCRW" height="120" />
  </a>
  <p align="center">The web scraper built for AI agents. Single binary. Zero config.</p>
  <p align="center">
    <a href="https://crates.io/crates/crw-server"><img src="https://img.shields.io/crates/v/crw-server.svg" alt="crates.io"></a>
    <a href="https://github.com/us/crw/actions"><img src="https://github.com/us/crw/workflows/CI/badge.svg" alt="CI"></a>
    <a href="LICENSE"><img src="https://img.shields.io/badge/license-AGPL--3.0-blue.svg" alt="License"></a>
    <a href="https://github.com/us/crw/stargazers"><img src="https://img.shields.io/github/stars/us/crw?style=social" alt="GitHub Stars"></a>
    <a href="https://fastcrw.com"><img src="https://img.shields.io/badge/Managed%20Cloud-fastcrw.com-blueviolet" alt="fastcrw.com"></a>
    <a href="https://discord.gg/kkFh2SC8"><img src="https://img.shields.io/badge/Discord-Join%20Community-7289da?logo=discord&logoColor=white" alt="Discord"></a>
    <br>
    <a href="https://www.producthunt.com/products/fastcrw?utm_source=badge-featured&utm_medium=badge&utm_campaign=badge-fastcrw" target="_blank" rel="noopener noreferrer"><img src="https://api.producthunt.com/widgets/embed-image/v1/featured.svg?post_id=1116966&theme=light&t=1775671073751" alt="fastCRW - Search + scrape live web results for AI agents | Product Hunt" width="250" height="54" /></a>
  </p>
  <p align="center">
    Works with: Claude Code · Cursor · Windsurf · Cline · Copilot · Continue.dev · Codex
  </p>
  <p align="center">
    <a href="#install">Install</a> &bull;
    <a href="#choose-your-mode">Modes</a> &bull;
    <a href="https://docs.fastcrw.com/#rest-api">API Reference</a> &bull;
    <a href="https://docs.fastcrw.com/#mcp">MCP Integration</a> &bull;
    <a href="https://docs.fastcrw.com/#js-rendering">JS Rendering</a> &bull;
    <a href="https://fastcrw.com">Cloud</a> &bull;
    <a href="https://discord.gg/kkFh2SC8">Discord</a>
  </p>
  <p align="center">
    <b>English</b> | <a href="README.zh-CN.md">中文</a>
  </p>
</p>

---

> **Don't want to self-host?** [fastcrw.com](https://fastcrw.com) — managed cloud with global proxy network, auto-scaling, web search, and dashboard. Same API, zero infra. [Get 500 free credits →](https://fastcrw.com)

## Scrape any URL in one command

```bash
crw example.com
```

```
# Example Domain

This domain is for use in illustrative examples in documents.
You may use this domain in literature without prior coordination or asking for permission.

[More information...](https://www.iana.org/domains/example)
```

Markdown output, no server, no config. [Install →](#install)

## Install

### CLI (`crw`) — scrape URLs from your terminal

```bash
# Homebrew:
brew install us/crw/crw

# APT (Debian/Ubuntu):
curl -fsSL https://apt.fastcrw.com/gpg.key | sudo gpg --dearmor -o /usr/share/keyrings/crw.gpg
echo "deb [signed-by=/usr/share/keyrings/crw.gpg] https://apt.fastcrw.com stable main" | sudo tee /etc/apt/sources.list.d/crw.list
sudo apt update && sudo apt install crw

# One-line install (auto-detects OS & arch):
curl -fsSL https://raw.githubusercontent.com/us/crw/main/install.sh | CRW_BINARY=crw sh

# Cargo:
cargo install crw-cli
```

### MCP Server (`crw-mcp`) — give AI agents web scraping tools

```bash
# Homebrew:
brew install us/crw/crw-mcp

# APT (Debian/Ubuntu — add repo once, see CLI section above):
sudo apt install crw-mcp

# One-line install:
curl -fsSL https://raw.githubusercontent.com/us/crw/main/install.sh | sh

# npm (zero install):
npx crw-mcp

# Python:
pip install crw

# Cargo:
cargo install crw-mcp

# Docker:
docker run -i ghcr.io/us/crw crw-mcp
```

> Listed on the [MCP Registry](https://registry.modelcontextprotocol.io/?q=crw)

### API Server (`crw-server`) — Firecrawl-compatible REST API

```bash
# Homebrew:
brew install us/crw/crw-server

# APT (Debian/Ubuntu — add repo once, see CLI section above):
sudo apt install crw-server

# One-line install:
curl -fsSL https://raw.githubusercontent.com/us/crw/main/install.sh | CRW_BINARY=crw-server sh

# Cargo:
cargo install crw-server

# Docker:
docker run -p 3000:3000 ghcr.io/us/crw
```

## Choose Your Mode

| | CLI (`crw`) | MCP (`crw-mcp`) | Server (`crw-server`) | Docker |
|---|---|---|---|---|
| **Use case** | Terminal scraping | AI agent tools | REST API backend | Containerized deploy |
| **Server needed?** | No | No (embedded) | Yes (`:3000`) | Yes (`:3000`) |
| **JS rendering** | `--js` (auto-detect)^1 | Auto-detect + auto-download^1 | `crw-server setup`^2 | Included (sidecar) |
| **Single URL scrape** | Yes | Yes | Yes | Yes |
| **Web search** | — | Cloud only^3 | — | — |
| **Async crawl** | — | Yes | Yes | Yes |
| **URL mapping** | — | Yes | Yes | Yes |
| **REST API** | — | — | Yes (Firecrawl-compat) | Yes (Firecrawl-compat) |
| **MCP protocol** | — | Yes (stdio + HTTP) | HTTP only | HTTP only |
| **Output** | stdout | MCP protocol | JSON responses | JSON responses |
| **LLM extraction** | — | Yes | Yes | Yes |

> ^1 **CLI + MCP:** Same auto-detect chain — LightPanda in PATH → `~/.crw/lightpanda` (auto-downloads if missing) → Chrome/Chromium on system → LightPanda Docker container. Falls back to HTTP-only if no browser found. CLI requires `--js` flag; MCP activates automatically. Both respect `CRW_CDP_URL` env var for manual override.
>
> ^2 **Server:** `crw-server setup` downloads LightPanda and creates `config.local.toml`. Start LightPanda separately before running the server. With Docker Compose, LightPanda runs as a sidecar automatically.
>
> ^3 **Web search:** `crw_search` is available only when connected to [fastcrw.com](https://fastcrw.com) cloud (`CRW_API_URL` set). In embedded/self-hosted mode, use `crw_map` for site discovery instead.

## Quick Start

**CLI:**

```bash
crw example.com                                   # markdown to stdout
crw example.com --format html                     # HTML output
crw example.com --format links                    # extract all links
crw example.com --js                              # with JS rendering (auto-detects browser)
crw example.com --css 'article' --raw             # extract specific elements
```

**MCP (AI agents — recommended):**

```bash
# Local (embedded — no server needed):
claude mcp add crw -- npx crw-mcp

# Cloud (fastcrw.com — includes web search):
claude mcp add -e CRW_API_URL=https://fastcrw.com/api -e CRW_API_KEY=your-key crw -- npx crw-mcp
```

> **Local mode** gives you `crw_scrape`, `crw_crawl`, `crw_map` tools. **Cloud mode** adds `crw_search` for web search powered by [fastcrw.com](https://fastcrw.com). For Cursor, Windsurf, Cline, and other MCP clients, see [MCP Server](#mcp-server).

**Self-hosted server:**

```bash
crw-server                    # start API server on :3000
crw-server setup              # optional: set up JS rendering (downloads LightPanda)
```

**Docker Compose (with JS rendering):**

```bash
docker compose up
```

**Scrape a page via API:**

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'
```

```json
{
  "success": true,
  "data": {
    "markdown": "# Example Domain\nThis domain is for use in ...",
    "metadata": {
      "title": "Example Domain",
      "sourceURL": "https://example.com",
      "statusCode": 200,
      "elapsedMs": 32
    }
  }
}
```

**Cloud (no setup):**

```bash
curl -X POST https://fastcrw.com/api/v1/scrape \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'
```

> Get your API key at [fastcrw.com](https://fastcrw.com) — 500 free credits included.

## Why CRW?

CRW gives you Firecrawl's API with a fraction of the resource usage. No runtime dependencies, no Redis, no Node.js — just a single binary you can deploy anywhere.

| Metric | CRW (self-hosted) | fastcrw.com (cloud) | Firecrawl | Tavily | Crawl4AI |
|---|---|---|---|---|---|
| **Coverage (1K URLs)** | **92.0%** | **92.0%** | 77.2% | — | — |
| **Avg Scrape Latency** | **833ms** | **833ms** | 4,600ms | — | — |
| **Avg Search Latency** | **880ms** | **880ms** | 954ms | 2,000ms | — |
| **Search Win Rate** | **73/100** | **73/100** | 25/100 | 2/100 | — |
| **Idle RAM** | 6.6 MB | 0 (managed) | ~500 MB+ | — (cloud) | — |
| **Cold start** | 85 ms | 0 (always-on) | 30–60 s | — | — |
| **Self-hosting** | **Single binary** | — | Multi-container | No | Python + Playwright |
| **Cost / 1K scrapes** | **$0** (self-hosted) | From $13/mo | $0.83–5.33 | — | $0 |
| **License** | AGPL-3.0 | Managed | AGPL-3.0 | Proprietary | Apache-2.0 |

<details>
<summary><b>Full benchmark details</b></summary>

**CRW vs Firecrawl** — Tested on [Firecrawl scrape-content-dataset-v1](https://huggingface.co/datasets/firecrawl/scrape-content-dataset-v1) (1,000 real-world URLs, JS rendering enabled):
- CRW covers **92%** of URLs vs Firecrawl's **77.2%** — 15 percentage points higher
- CRW is **5.5x faster** on average (833ms vs 4,600ms)
- CRW uses **~75x less RAM** at idle (6.6 MB vs ~500 MB+)
- Firecrawl requires 5 containers (Node.js, Redis, PostgreSQL, RabbitMQ, Playwright) — CRW is a single binary

**Firecrawl independent review** — [Scrapeway benchmark](https://scrapeway.com/web-scraping-api/firecrawl): 64.3% success rate, $5.11/1K scrapes, 0% on LinkedIn/Twitter.

**Resource comparison:**

| Metric | CRW | Firecrawl |
|---|---|---|
| Min RAM | ~7 MB | 4 GB |
| Recommended RAM | ~64 MB (under load) | 8–16 GB |
| Docker images | single ~8 MB binary | ~2–3 GB total |
| Cold start | 85 ms | 30–60 seconds |
| Containers needed | 1 (+optional sidecar) | 5 |

</details>

## Features

- **MCP server** — built-in stdio + HTTP transport for Claude Code, Cursor, Windsurf, and any MCP client
- **Firecrawl-compatible API** — same endpoint family and familiar request/response ergonomics
- **6 output formats** — markdown, HTML, cleaned HTML, raw HTML, plain text, links, structured JSON
- **LLM structured extraction** — send a JSON schema, get validated structured data back (Anthropic tool_use + OpenAI function calling)
- **JS rendering** — auto-detect SPAs with shell heuristics, render via LightPanda, Playwright, or Chrome (CDP)
- **BFS crawler** — async crawl with rate limiting, robots.txt, sitemap support, concurrent jobs
- **Security** — SSRF protection (private IPs, cloud metadata, IPv6), constant-time auth, dangerous URI filtering
- **Docker ready** — multi-stage build with LightPanda sidecar
- **CSS selector & XPath** — extract specific DOM elements before Markdown conversion
- **Chunking & filtering** — split content into topic/sentence/regex chunks; rank by BM25 or cosine similarity
- **Stealth mode** — browser-like UA rotation and header injection to reduce bot detection
- **Per-request proxy** — override the global proxy per scrape request
- **Web search** — search the web and optionally scrape results, with news and image support (cloud only via [fastcrw.com](https://fastcrw.com))

## Use Cases

- **RAG pipelines** — crawl websites and extract structured data for vector databases
- **AI agents** — give Claude Code or Claude Desktop web scraping tools via MCP
- **Content monitoring** — periodic crawl with LLM extraction to track changes
- **Data extraction** — combine CSS selectors + LLM to extract any schema from any page
- **Web archiving** — full-site BFS crawl to markdown

## API Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| `POST` | `/v1/scrape` | Scrape a single URL, optionally with LLM extraction |
| `POST` | `/v1/crawl` | Start async BFS crawl (returns job ID) |
| `GET` | `/v1/crawl/:id` | Check crawl status and retrieve results |
| `DELETE` | `/v1/crawl/:id` | Cancel a running crawl job |
| `POST` | `/v1/map` | Discover all URLs on a site |
| `POST` | `/v1/search` | **Cloud only** — Web search with optional content scraping |
| `GET` | `/health` | Health check (no auth required) |
| `POST` | `/mcp` | Streamable HTTP MCP transport |

## LLM Structured Extraction

Send a JSON schema with your scrape request and CRW returns validated structured data using LLM function calling.

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com/product",
    "formats": ["json"],
    "jsonSchema": {
      "type": "object",
      "properties": {
        "name": { "type": "string" },
        "price": { "type": "number" }
      },
      "required": ["name", "price"]
    }
  }'
```

- **Anthropic** — uses `tool_use` with `input_schema` for extraction
- **OpenAI** — uses function calling with `parameters` schema
- **Validation** — LLM output is validated against your JSON schema before returning

Configure the LLM provider in your config:

```toml
[extraction.llm]
provider = "anthropic"        # "anthropic" or "openai"
api_key = "sk-..."            # or CRW_EXTRACTION__LLM__API_KEY env var
model = "claude-sonnet-4-20250514"
```

## MCP Server

CRW works as an MCP tool server for any AI assistant that supports MCP. It provides 4 tools in local/embedded mode (`crw_scrape`, `crw_crawl`, `crw_check_crawl_status`, `crw_map`) and adds `crw_search` when connected to [fastcrw.com](https://fastcrw.com) cloud.

> Also available on the [MCP Registry](https://registry.modelcontextprotocol.io/?q=crw)

**Install:**

```bash
# npm (zero install):
npx crw-mcp

# Python:
pip install crw

# Direct binary (no package manager):
curl -fsSL https://github.com/us/crw/releases/latest/download/crw-mcp-darwin-arm64.tar.gz | tar xz
# Replace darwin-arm64 with your platform: darwin-x64, linux-x64, linux-arm64, win32-x64, win32-arm64

# Cargo:
cargo install crw-mcp

# Docker:
docker run -i ghcr.io/us/crw crw-mcp
```

### Claude Code

```bash
# Embedded (self-contained, no server needed):
claude mcp add crw -- npx crw-mcp

# Cloud (fastcrw.com):
claude mcp add -e CRW_API_URL=https://fastcrw.com/api -e CRW_API_KEY=your-key crw -- npx crw-mcp
```

### Cloud / Hosted Mode

Connect to [fastcrw.com](https://fastcrw.com) instead of running a local engine. Cloud mode unlocks **`crw_search`** — web search powered by fastCRW, in addition to all local tools.

| Mode | Tools | Requires |
|------|-------|----------|
| **Embedded** (default) | `crw_scrape`, `crw_crawl`, `crw_check_crawl_status`, `crw_map` | Nothing |
| **Cloud** (`CRW_API_URL` set) | All embedded tools + **`crw_search`** | [fastcrw.com](https://fastcrw.com) API key |

Set `CRW_API_URL` and `CRW_API_KEY` in any MCP client config:

```json
{
  "mcpServers": {
    "crw": {
      "command": "npx",
      "args": ["crw-mcp"],
      "env": {
        "CRW_API_URL": "https://fastcrw.com/api",
        "CRW_API_KEY": "your-api-key"
      }
    }
  }
}
```

> Get your API key at [fastcrw.com](https://fastcrw.com). The same `env` block works in Claude Desktop, Cursor, Windsurf, Cline, and any other MCP client below.

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
      "command": "npx",
      "args": ["crw-mcp"]
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
      "command": "npx",
      "args": ["crw-mcp"]
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
      "command": "npx",
      "args": ["crw-mcp"]
    }
  }
}
```

### Cline (VS Code)

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

### Continue.dev (VS Code / JetBrains)

Edit `~/.continue/config.yaml`:

```yaml
mcpServers:
  - name: crw
    command: npx
    args:
      - crw-mcp
```

### OpenAI Codex CLI

Edit `~/.codex/config.toml`:

```toml
[mcp_servers.crw]
command = "npx"
args = ["crw-mcp"]
```

### Other MCP Clients

Any MCP-compatible client can connect to CRW using the standard JSON format:

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

> **Tip:** The stdio binary (`crw-mcp`) works with any client. For clients that support HTTP transport, use `http://localhost:3000/mcp` directly — no binary needed. To enable cloud features like web search, add the `env` block from [Cloud / Hosted Mode](#cloud--hosted-mode) to any config above.

See the full [MCP setup guide](docs/docs/mcp-clients.md) for detailed instructions, auth configuration, and platform comparison.

## JS Rendering

CRW auto-detects SPAs by analyzing the initial HTML response for shell heuristics (empty body, framework markers). When a SPA is detected, it renders the page via a headless browser.

**Quick setup (recommended):**

```bash
crw-server setup
```

This automatically downloads the LightPanda binary to `~/.local/bin/` and creates a `config.local.toml` with the correct renderer settings. Then start LightPanda and CRW:

```bash
lightpanda serve --host 127.0.0.1 --port 9222 &
crw-server
```

**Supported renderers:**

| Renderer | Protocol | Best for |
|----------|----------|----------|
| LightPanda | CDP over WebSocket | Low-resource environments (default) |
| Playwright | CDP over WebSocket | Full browser compatibility |
| Chrome | CDP over WebSocket | Existing Chrome infrastructure |

Renderer mode is configured via `renderer.mode`: `auto` (default), `lightpanda`, `playwright`, `chrome`, or `none`.

With Docker Compose, LightPanda runs as a sidecar — no extra setup needed:

```bash
docker compose up
```

## Architecture

```
┌─────────────────────────────────────────────┐
│                 crw-server                  │
│         Axum HTTP API + Auth + MCP          │
├──────────┬──────────┬───────────────────────┤
│ crw-crawl│crw-extract│    crw-renderer      │
│ BFS crawl│ HTML→MD   │  HTTP + CDP(WS)      │
│ robots   │ LLM/JSON  │  LightPanda/Chrome   │
│ sitemap  │ clean/read│  auto-detect SPA     │
├──────────┴──────────┴───────────────────────┤
│                 crw-core                    │
│        Types, Config, Errors                │
└─────────────────────────────────────────────┘
```

## Configuration

CRW uses layered TOML configuration with environment variable overrides:

1. `config.default.toml` — built-in defaults
2. `config.local.toml` — local overrides (or set `CRW_CONFIG=myconfig`)
3. Environment variables — `CRW_` prefix with `__` separator (e.g. `CRW_SERVER__PORT=8080`)

```toml
[server]
host = "0.0.0.0"
port = 3000
rate_limit_rps = 10        # Max requests/second (global). 0 = unlimited.

[renderer]
mode = "auto"  # auto | lightpanda | playwright | chrome | none

[crawler]
max_concurrency = 10
requests_per_second = 10.0
respect_robots_txt = true

[auth]
# api_keys = ["fc-key-1234"]
```

See [full configuration reference](https://docs.fastcrw.com/#configuration) for all options.

## Integration Examples

**Python:**

```python
import requests

response = requests.post("http://localhost:3000/v1/scrape", json={
    "url": "https://example.com",
    "formats": ["markdown", "links"]
})
data = response.json()["data"]
print(data["markdown"])
```

**Node.js:**

```javascript
const response = await fetch("http://localhost:3000/v1/scrape", {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({
    url: "https://example.com",
    formats: ["markdown", "links"]
  })
});
const { data } = await response.json();
console.log(data.markdown);
```

**CrewAI** ([`crewai-crw`](https://pypi.org/project/crewai-crw/) — CRW tools for CrewAI):

```bash
pip install crewai-crw
```

```python
from crewai import Agent, Task, Crew
from crewai_crw import CrwScrapeWebsiteTool, CrwCrawlWebsiteTool, CrwMapWebsiteTool

scrape_tool = CrwScrapeWebsiteTool()  # uses localhost:3000 by default

researcher = Agent(
    role="Web Researcher",
    goal="Research and summarize information from websites",
    backstory="Expert at extracting key information from web pages",
    tools=[scrape_tool],
)

task = Task(
    description="Scrape https://example.com and summarize the content",
    expected_output="A summary of the page content",
    agent=researcher,
)

crew = Crew(agents=[researcher], tasks=[task])
result = crew.kickoff()
```

**LangChain** ([`langchain-crw`](https://pypi.org/project/langchain-crw/) — CRW document loader):

```bash
pip install langchain-crw
```

```python
from langchain_crw import CrwLoader

# Self-hosted (default: localhost:3000)
loader = CrwLoader(url="https://example.com", mode="scrape")
docs = loader.load()
print(docs[0].page_content)  # clean markdown

# Cloud (fastcrw.com)
loader = CrwLoader(
    url="https://example.com",
    api_url="https://fastcrw.com/api",
    api_key="crw_live_...",
    mode="crawl",
    params={"max_depth": 3, "max_pages": 50},
)
docs = loader.load()
```

> **More integrations (PRs pending):** [Flowise](https://github.com/FlowiseAI/Flowise/pull/6066) · [Agno](https://github.com/agno-agi/agno/pull/7183) · [n8n](https://fastcrw.com/blog/n8n-web-scraping-crw) · [All integrations](https://fastcrw.com/docs#integrations)

## Docker

**Pre-built image from GHCR:**

```bash
docker pull ghcr.io/us/crw:latest
docker run -p 3000:3000 ghcr.io/us/crw:latest
```

**Docker Compose (with JS rendering sidecar):**

```bash
docker compose up
```

This starts CRW on port `3000` with LightPanda as a JS rendering sidecar on port `9222`. CRW auto-connects to LightPanda for SPA rendering.

## Benchmark

### Search (100 queries, concurrent)

Benchmarked against Firecrawl and Tavily across 100 search queries in 10 categories — all run concurrently via `Promise.all`:

| Metric | CRW | Firecrawl | Tavily |
|---|---|---|---|
| **Avg Latency** | **880ms** | 954ms | 2,000ms |
| **Median Latency** | **785ms** | 932ms | 1,724ms |
| **P95 Latency** | **1,433ms** | 1,343ms | 3,534ms |
| **Win Rate** | **73/100** | 25/100 | 2/100 |

CRW is **2.3x faster than Tavily** and won 73% of latency races. [Full search benchmark →](https://fastcrw.com/benchmarks/tavily-search)

### Scrape (1,000 URLs, JS rendering enabled)

Tested on [Firecrawl's scrape-content-dataset-v1](https://huggingface.co/datasets/firecrawl/scrape-content-dataset-v1) (1,000 real-world URLs, JS rendering enabled):

| Metric | CRW | Firecrawl v2.5 | Crawl4AI |
|---|---|---|---|
| **Coverage** | **92.0%** | 77.2% | — |
| **Avg Latency** | **833ms** | 4,600ms | — |
| **P50 Latency** | **446ms** | — | — |
| **Noise Rejection** | **88.4%** | noise 6.8% | noise 11.3% |
| **Cost / 1,000 scrapes** | **$0** (self-hosted) | $0.83–5.33 | $0 |
| **Idle RAM** | **6.6 MB** | ~500 MB+ | — |

CRW hits a sweet spot: **the fastest self-hostable scraper** with **Firecrawl API compatibility** and **~75x less RAM**. Unlike Crawl4AI (Python + Playwright), CRW ships as a single Rust binary with no runtime dependencies.

Run the benchmark yourself:

```bash
uv pip install datasets aiohttp
uv run python bench/run_bench.py
```

## Crates

| Crate | Description | |
|-------|-------------|-|
| [`crw-core`](crates/crw-core) | Core types, config, and error handling | [![crates.io](https://img.shields.io/crates/v/crw-core.svg)](https://crates.io/crates/crw-core) |
| [`crw-renderer`](crates/crw-renderer) | HTTP + CDP browser rendering engine | [![crates.io](https://img.shields.io/crates/v/crw-renderer.svg)](https://crates.io/crates/crw-renderer) |
| [`crw-extract`](crates/crw-extract) | HTML → markdown/plaintext extraction | [![crates.io](https://img.shields.io/crates/v/crw-extract.svg)](https://crates.io/crates/crw-extract) |
| [`crw-crawl`](crates/crw-crawl) | Async BFS crawler with robots.txt & sitemap | [![crates.io](https://img.shields.io/crates/v/crw-crawl.svg)](https://crates.io/crates/crw-crawl) |
| [`crw-server`](crates/crw-server) | Axum API server (Firecrawl-compatible) | [![crates.io](https://img.shields.io/crates/v/crw-server.svg)](https://crates.io/crates/crw-server) |
| [`crw-cli`](crates/crw-cli) | Standalone CLI (`crw` binary, no server needed) | [![crates.io](https://img.shields.io/crates/v/crw-cli.svg)](https://crates.io/crates/crw-cli) |
| [`crw-mcp`](crates/crw-mcp) | MCP stdio proxy binary | [![crates.io](https://img.shields.io/crates/v/crw-mcp.svg)](https://crates.io/crates/crw-mcp) |

See [docs/crates.md](docs/crates.md) for usage examples and `cargo add` instructions.

## Documentation

Full documentation: **[docs.fastcrw.com](https://docs.fastcrw.com/#introduction)**

- [Getting Started](https://docs.fastcrw.com/#quick-start)
- [Configuration](https://docs.fastcrw.com/#configuration)
- [API Reference](https://docs.fastcrw.com/#rest-api)
- [MCP Server](https://docs.fastcrw.com/#mcp)
- [JS Rendering](https://docs.fastcrw.com/#js-rendering)
- [Architecture](https://docs.fastcrw.com/#architecture)

## Security

CRW includes built-in protections against common web scraping attack vectors:

- **SSRF protection** — all URL inputs (REST API + MCP) are validated against private/internal networks:
  - Loopback (`127.0.0.0/8`, `::1`, `localhost`)
  - Private IPs (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`)
  - Link-local / cloud metadata (`169.254.0.0/16` — blocks AWS/GCP metadata endpoints)
  - IPv6 mapped addresses (`::ffff:127.0.0.1`), link-local (`fe80::`), ULA (`fc00::/7`)
  - Non-HTTP schemes (`file://`, `ftp://`, `gopher://`, `data:`)
- **Auth** — optional Bearer token with constant-time comparison (no length or key-index leakage)
- **robots.txt** — respects `Allow`/`Disallow` with wildcard patterns (`*`, `$`) and RFC 9309 specificity
- **Rate limiting** — configurable per-second request cap with token-bucket algorithm (returns 429 with `error_code`)
- **Resource limits** — max body size (1 MB), max crawl depth (10), max pages (1000), max discovered URLs (5000)

## Community

Don't be shy — [join us on Discord](https://discord.gg/kkFh2SC8)! Ask questions, share what you're building, report bugs, or just hang out.

## Contributing

Contributions are welcome! Please open an issue or submit a pull request.

1. Fork the repository
2. Install pre-commit hooks: `make hooks`
3. Create your feature branch (`git checkout -b feat/my-feature`)
4. Commit your changes (`git commit -m 'feat: add my feature'`)
5. Push to the branch (`git push origin feat/my-feature`)
6. Open a Pull Request

The pre-commit hook runs the same checks as CI (`cargo fmt`, `cargo clippy`, `cargo test`). You can also run them manually with `make check`.

## Changelog

[Full changelog →](CHANGELOG.md)

## License

CRW is open-source under [AGPL-3.0](LICENSE). For a managed version without AGPL obligations, see [fastcrw.com](https://fastcrw.com).
