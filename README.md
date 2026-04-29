<a name="readme-top"></a>
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
  </p>
  <p align="center">
    <a href="https://twitter.com/fastcrw">
      <img src="https://img.shields.io/badge/Follow%20on%20X-000000?style=for-the-badge&logo=x&logoColor=white" alt="Follow on X" />
    </a>
    <a href="https://www.linkedin.com/company/fastcrw">
      <img src="https://img.shields.io/badge/Follow%20on%20LinkedIn-0077B5?style=for-the-badge&logo=linkedin&logoColor=white" alt="Follow on LinkedIn" />
    </a>
    <a href="https://discord.gg/kkFh2SC8">
      <img src="https://img.shields.io/badge/Join%20our%20Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white" alt="Join our Discord" />
    </a>
  </p>
  <p align="center">
    <a href="https://www.producthunt.com/products/fastcrw?utm_source=badge-featured&utm_medium=badge&utm_campaign=badge-fastcrw" target="_blank" rel="noopener noreferrer"><img src="https://api.producthunt.com/widgets/embed-image/v1/featured.svg?post_id=1116966&theme=light&t=1775671073751" alt="fastCRW - Search + scrape live web results for AI agents | Product Hunt" width="250" height="54" /></a>
  </p>
  <p align="center">
    Works with: Claude Code · Cursor · Windsurf · Cline · Copilot · Continue.dev · Codex · Gemini CLI
  </p>
  <p align="center">
    <a href="#quick-start">Quick Start</a> &bull;
    <a href="#connect-to-ai-agents--mcp-skill-onboarding">AI Agents</a> &bull;
    <a href="#benchmark">Benchmarks</a> &bull;
    <a href="https://docs.fastcrw.com/#rest-api">API Reference</a> &bull;
    <a href="https://fastcrw.com">Cloud</a> &bull;
    <a href="https://discord.gg/kkFh2SC8">Discord</a>
  </p>
  <p align="center">
    <b>English</b> | <a href="README.zh-CN.md">中文</a>
  </p>
</p>

---

## What's New

### [0.4.1](https://github.com/us/crw/compare/v0.4.0...v0.4.1) (2026-04-28)


### Features

* add per-request renderer field for scrape and crawl APIs ([#29](https://github.com/us/crw/issues/29)) ([f1e0b63](https://github.com/us/crw/commit/f1e0b63fd28be0ceb38342086a309f92bbbc1e53))
* **crw-browse:** add interactive browser MCP server with phase-2 tools ([e78879d](https://github.com/us/crw/commit/e78879db18c7c4b3df2a4984349a65b4493b1cda))
* honor renderer mode and force_js in config (fixes [#28](https://github.com/us/crw/issues/28)) ([b76e473](https://github.com/us/crw/commit/b76e473facbce08a841ef8bd9fdfac97a552a8fd))


### Bug Fixes

* detect failed JS renders and fail over to next renderer ([fca8fd5](https://github.com/us/crw/commit/fca8fd5cadb4fa3c96bf5a315f96ab6d1e63989c))
* **docs:** use absolute logo paths in site.config.js ([c5c9321](https://github.com/us/crw/commit/c5c93215561094a35038ab6af2b21e91c16199f4))
* **docs:** use absolute paths for logo and favicon assets ([cdb1451](https://github.com/us/crw/commit/cdb14517da6425c105b95749dec35bbc9e977f5e))

### [0.4.0](https://github.com/us/crw/compare/v0.3.6...v0.4.0) (2026-04-22)


### Features

* add crw-browse MCP server, SOCKS5 proxy, extract mcp-proto ([9a53753](https://github.com/us/crw/commit/9a53753baf6d87272bd2417fc87102a8ed34d41b))


### Miscellaneous

* release 0.4.0 ([e15fc74](https://github.com/us/crw/commit/e15fc74cf0dfc7c02ca7e6b82258aeff57f74f17))

### [0.3.6](https://github.com/us/crw/compare/v0.3.5...v0.3.6) (2026-04-21)


### Features

* **ci:** add Google Indexing API notification for docs changes ([3b5a340](https://github.com/us/crw/commit/3b5a3404e91a1d776275ac312ad08cad86339a98))
* **docs:** generate static HTML pages for SEO indexability ([7b321c0](https://github.com/us/crw/commit/7b321c0a26cea0da32d42e952f6327b468bdb099))


### Bug Fixes

* **ci:** trigger release workflow after release-please creates tag ([27f2b67](https://github.com/us/crw/commit/27f2b67d0b9db4f7b1bacc6c901e9c92131a3a95))
* **mcp:** bump npm optionalDependencies from 0.3.0 to 0.3.5 ([0e363e0](https://github.com/us/crw/commit/0e363e0fc512eb18bcb8284a9723b00f50e2dfd0))
* **renderer:** detect loading placeholders and poll for content stability ([d3b642b](https://github.com/us/crw/commit/d3b642b2736b4568fa8a3502e521b8bede60317f))

[Full changelog →](CHANGELOG.md)

---

# fastCRW — Open Source Web Scraping API for AI Agents

**Power AI agents with clean web data.** Single Rust binary, zero config, Firecrawl-compatible API. The open-source Firecrawl alternative you can self-host for free — or use our [managed cloud](https://fastcrw.com).

> **Don't want to self-host?** [**Sign up free →**](https://fastcrw.com) — managed cloud with global proxy network, web search, and dashboard. Same API, zero infra. **500 free credits, no credit card required.**

---

## Why CRW? — Firecrawl & Crawl4AI Alternative

- **Single binary, 6 MB RAM** — no Redis, no Node.js, no containers. Firecrawl needs 5 containers and 4 GB+. Crawl4AI requires Python + Playwright
- **5.5x faster than Firecrawl** — 833ms avg vs 4,600ms ([see benchmarks](#benchmark)). P50 at 446ms
- **73/100 search win rate** — beats Firecrawl (25/100) and Tavily (2/100) in head-to-head benchmarks
- **Free self-hosting** — $0/1K scrapes vs Firecrawl's $0.83–5.33. No infra, no cold starts (85ms). No API key required for local mode
- **Agent ready** — add to any MCP client in one command. Embedded mode: no server needed
- **Firecrawl-compatible API** — drop-in replacement. Same `/v1/scrape`, `/v1/crawl`, `/v1/map` endpoints. HTML to markdown, structured data extraction, website crawler — all built-in
- **Built for RAG pipelines** — clean LLM-ready markdown output for vector databases and AI data ingestion
- **Open source** — AGPL-3.0, developed transparently. [Join our community](https://discord.gg/kkFh2SC8)

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

---

## Web Scraping & Crawling Features

**Core**

| Feature | Description |
|---------|-------------|
| [**Scrape**](#scrape) | Convert any URL to markdown, HTML, JSON, or links |
| [**Crawl**](#crawl) | Async BFS website crawler with rate limiting |
| [**Map**](#map) | Discover all URLs on a site instantly |
| [**Search**](#search) | Web search + content scraping (cloud) |

**More**

| Feature | Description |
|---------|-------------|
| [**LLM Extraction**](#llm-structured-extraction) | Send a JSON schema, get validated structured data back |
| [**JS Rendering**](#js-rendering) | Auto-detect SPAs, render via LightPanda or Chrome |
| [**CLI**](#cli) | Scrape any URL from your terminal — no server needed |
| [**MCP Server**](#mcp-server-for-ai-agents) | Built-in stdio + HTTP transport for any AI agent |

**Use Cases:** RAG pipelines · AI agent web access · content monitoring · data extraction · HTML to markdown conversion · web archiving

---

## Quick Start

```bash
# Install:
curl -fsSL https://raw.githubusercontent.com/us/crw/main/install.sh | CRW_BINARY=crw sh

# Scrape:
crw example.com

# Add to Claude Code (local):
claude mcp add crw -- npx crw-mcp
# Add to Claude Code (cloud — includes web search, 500 free credits at fastcrw.com):
claude mcp add -e CRW_API_URL=https://fastcrw.com/api -e CRW_API_KEY=your-key crw -- npx crw-mcp
```

> Or: `pip install crw` (Python SDK) · `npx crw-mcp` (zero install) · `brew install us/crw/crw` (Homebrew) · [All install options →](https://docs.fastcrw.com/installation/)

### Scrape

Convert any URL to clean markdown, HTML, or structured JSON.

```python
from crw import CrwClient

client = CrwClient(api_url="https://fastcrw.com/api", api_key="YOUR_API_KEY")  # local: CrwClient()
result = client.scrape("https://example.com")
print(result["markdown"])
```

> **Local mode:** `CrwClient()` with no arguments runs a self-contained scraping engine — no server, no API key, no setup. The SDK automatically downloads the `crw-mcp` binary on first use.

<details>
<summary><b>CLI / cURL</b></summary>

**CLI:**
```bash
crw example.com
crw example.com --format html
crw example.com --js --css 'article'
```

**Self-hosted** (`crw-server` running on `:3000`):
```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'
```

**Cloud:**
```bash
curl -X POST https://fastcrw.com/api/v1/scrape \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'
```
</details>

Output:
```
# Example Domain

This domain is for use in illustrative examples in documents.
You may use this domain in literature without prior coordination.
```

#### Renderer selection & response metadata

CRW picks between three rendering backends per request:

- **`http`** (1 credit) — plain HTTP fetch. Used for static pages.
- **`lightpanda`** (1 credit) — lightweight JS renderer for most SPAs.
- **`chrome`** (2 credits) — full Chromium for sites where LightPanda's hydration crashes (e.g. some Next.js App Router pages).

By default the engine auto-selects, learns per-host preferences after repeated failures, and falls over chrome → lightpanda → http transparently. Pass `"renderer"` to pin one of `auto | http | lightpanda | chrome` (Firecrawl's `engine` is also accepted as an alias).

Every successful response includes routing metadata so callers can audit and debug:

```jsonc
{
  "data": {
    "markdown": "...",
    "renderDecision": {
      "kind": "failover",                 // autoDefault | autoPromoted | userPinned | failover | breakerSkipped
      "chain": ["lightpanda", "chrome"],  // renderers actually attempted
      "reason": "nextJsClientError"       // why the chain advanced
    },
    "creditCost": 2,
    "warnings": [
      "lightpanda returned a failed render (nextjs_client_error)"
    ],
    "metadata": { "renderedWith": "chrome", /* … */ }
  }
}
```

When you hard-pin a renderer that fails (e.g. `"renderer":"lightpanda"` on a hydration-crashing page), `success` stays `true` for protocol compatibility — but `data.warnings[]` carries an actionable hint suggesting `renderer="chrome"` or auto mode. Clients should surface the warnings array.

### Crawl

Scrape all pages of a website asynchronously.

```python
from crw import CrwClient

client = CrwClient(api_url="https://fastcrw.com/api", api_key="YOUR_API_KEY")  # local: CrwClient()
pages = client.crawl("https://docs.example.com", max_depth=2, max_pages=50)
for page in pages:
    print(page["metadata"]["sourceURL"], page["markdown"][:80])
```

<details>
<summary><b>CLI / cURL</b></summary>

```bash
# Start crawl
curl -X POST http://localhost:3000/v1/crawl \
  -H "Content-Type: application/json" \
  -d '{"url": "https://docs.example.com", "maxDepth": 2, "maxPages": 50}'

# Check status (use job ID from above)
curl http://localhost:3000/v1/crawl/JOB_ID
```
</details>

### Map

Discover all URLs on a site instantly.

```python
from crw import CrwClient

client = CrwClient(api_url="https://fastcrw.com/api", api_key="YOUR_API_KEY")  # local: CrwClient()
urls = client.map("https://example.com")
print(urls)  # ["https://example.com", "https://example.com/about", ...]
```

<details>
<summary><b>cURL</b></summary>

```bash
curl -X POST http://localhost:3000/v1/map \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'
```
</details>

### Search

Search the web and get full page content from results.

```python
from crw import CrwClient

# Cloud only — requires fastcrw.com API key
client = CrwClient(api_url="https://fastcrw.com/api", api_key="YOUR_KEY")
results = client.search("open source web scraper 2026", limit=10)
```

> **Cloud only:** `search()` requires a [fastcrw.com](https://fastcrw.com) API key (**500 free credits, no credit card**). Local/embedded mode provides `scrape`, `crawl`, and `map`.

<details>
<summary><b>cURL</b></summary>

```bash
curl -X POST https://fastcrw.com/api/v1/search \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"query": "open source web scraper 2026", "limit": 10}'
```
</details>

### API Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| `POST` | `/v1/scrape` | Scrape a single URL, optionally with LLM extraction |
| `POST` | `/v1/crawl` | Start async BFS crawl (returns job ID) |
| `GET` | `/v1/crawl/:id` | Check crawl status and retrieve results |
| `DELETE` | `/v1/crawl/:id` | Cancel a running crawl job |
| `POST` | `/v1/map` | Discover all URLs on a site |
| `POST` | `/v1/search` | Web search with optional content scraping (cloud only) |
| `GET` | `/health` | Health check (no auth required) |
| `POST` | `/mcp` | Streamable HTTP MCP transport |

[Full API reference →](https://docs.fastcrw.com/#rest-api)

---

## Connect to AI Agents — MCP, Skill, Onboarding

Add CRW to any AI agent or MCP client in seconds.

### Skill

Install the CRW skill to all detected agents with one command:

```bash
npx crw-mcp init --all
```

Restart your agent after installing. Works with Claude Code, Cursor, Gemini CLI, Codex, OpenCode, and Windsurf.

### MCP Server for AI Agents

Add CRW to any MCP-compatible client:

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

> Works with Claude Desktop, Cursor, Windsurf, Cline, Continue.dev, and any MCP client.
>
> **Config file locations:** Claude Code — `claude mcp add` (no file edit). Claude Desktop — `~/Library/Application Support/Claude/claude_desktop_config.json`. Cursor — `.cursor/mcp.json`. Windsurf — `~/.codeium/windsurf/mcp_config.json`. [All clients →](https://docs.fastcrw.com/mcp-clients/)

**Cloud mode** — adds `crw_search` for web search:

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

### MCP Tools

| Mode | Tool | Description |
|------|------|-------------|
| **Embedded + Cloud** | `crw_scrape` | Scrape a URL → markdown, HTML, JSON, links |
| **Embedded + Cloud** | `crw_crawl` | Start async BFS crawl (returns job ID) |
| **Embedded + Cloud** | `crw_check_crawl_status` | Poll crawl job status (use with `crw_crawl`) |
| **Embedded + Cloud** | `crw_map` | Discover all URLs on a site |
| **Cloud only** | `crw_search` | Web search + optional content scraping |

> **Embedded mode** (default): no server, no API key, no setup — the MCP binary runs a self-contained scraping engine. **Cloud mode** (`CRW_API_URL` set): forwards calls to [fastcrw.com](https://fastcrw.com), adds `crw_search`.

[Full MCP docs →](https://docs.fastcrw.com/mcp/)

**Claude Code shortcut:**

```bash
# Local (embedded — no server, no API key):
claude mcp add crw -- npx crw-mcp

# Cloud (+ web search):
claude mcp add -e CRW_API_URL=https://fastcrw.com/api -e CRW_API_KEY=your-key crw -- npx crw-mcp
```

> Listed on the [MCP Registry](https://registry.modelcontextprotocol.io/?q=crw)

### Agent Onboarding

Are you an AI agent? Fetch this skill file to configure CRW for the user's coding agents. The SKILL.md contains tool descriptions, usage examples, authentication setup, and edge case handling:

```bash
curl -s https://fastcrw.com/agent-onboarding/SKILL.md
```

---

## Benchmark

### Search — CRW vs Firecrawl vs Tavily (100 queries, concurrent)

| Metric | CRW | Firecrawl | Tavily |
|---|---|---|---|
| **Avg Latency** | **880ms** | 954ms | 2,000ms |
| **Median Latency** | **785ms** | 932ms | 1,724ms |
| **Win Rate** | **73/100** | 25/100 | 2/100 |

CRW is **2.3x faster than Tavily** and won 73% of latency races. [Full search benchmark →](https://fastcrw.com/benchmarks/tavily-search)

### Scrape — CRW vs Firecrawl (1,000 URLs, JS rendering enabled)

Tested on [Firecrawl's scrape-content-dataset-v1](https://huggingface.co/datasets/firecrawl/scrape-content-dataset-v1):

| Metric | CRW | Firecrawl v2.5 |
|---|---|---|
| **Coverage** | **92.0%** | 77.2% |
| **Avg Latency** | **833ms** | 4,600ms |
| **P50 Latency** | **446ms** | — |
| **Noise Rejection** | **88.4%** | noise 6.8% |
| **Idle RAM** | **6.6 MB** | ~500 MB+ |
| **Cost / 1K scrapes** | **$0** (self-hosted) | $0.83–5.33 |

<details>
<summary><b>Resource comparison</b></summary>

| Metric | CRW | Firecrawl |
|---|---|---|
| Min RAM | ~7 MB | 4 GB |
| Recommended RAM | ~64 MB (under load) | 8–16 GB |
| Docker images | single ~8 MB binary | ~2–3 GB total |
| Cold start | 85 ms | 30–60 seconds |
| Containers needed | 1 (+optional sidecar) | 5 |

</details>

[Full benchmark details →](https://docs.fastcrw.com/introduction/#benchmarks)

Run the benchmark yourself:

```bash
pip install datasets aiohttp
python bench/run_bench.py
```

---

## Install

### MCP Server (`crw-mcp`) — recommended for AI agents

```bash
npx crw-mcp                           # zero install (npm)
pip install crw                        # Python SDK (auto-downloads binary)
brew install us/crw/crw-mcp            # Homebrew
cargo install crw-mcp                  # Cargo
docker run -i ghcr.io/us/crw crw-mcp  # Docker
```

### CLI (`crw`) — scrape URLs from your terminal

```bash
brew install us/crw/crw

# One-line install (auto-detects OS & arch):
curl -fsSL https://raw.githubusercontent.com/us/crw/main/install.sh | CRW_BINARY=crw sh

# APT (Debian/Ubuntu):
curl -fsSL https://apt.fastcrw.com/gpg.key | sudo gpg --dearmor -o /usr/share/keyrings/crw.gpg
echo "deb [signed-by=/usr/share/keyrings/crw.gpg] https://apt.fastcrw.com stable main" | sudo tee /etc/apt/sources.list.d/crw.list
sudo apt update && sudo apt install crw

cargo install crw-cli
```

### API Server (`crw-server`) — Firecrawl-compatible REST API

For serving multiple apps, other languages (Node.js, Go, Java), or as a shared microservice.

```bash
brew install us/crw/crw-server

# One-line install:
curl -fsSL https://raw.githubusercontent.com/us/crw/main/install.sh | CRW_BINARY=crw-server sh

# Docker:
docker run -p 3000:3000 ghcr.io/us/crw
```

Custom port:
```bash
CRW_SERVER__PORT=8080 crw-server                                       # env var
docker run -p 8080:8080 -e CRW_SERVER__PORT=8080 ghcr.io/us/crw       # Docker
```

**Docker Compose** ships with `lightpanda` enabled by default; `chrome` is opt-in to keep small VPS deploys lean (~500MB image + 1GB resident):

```bash
# baseline — http + lightpanda
docker compose up -d

# add chrome failover (recommended for production)
docker compose --profile heavy up -d
```

Without `--profile heavy`, the engine still serves all endpoints — chrome-required URLs will exhaust their lightpanda failover and surface `data.warnings[]` instead of routing to chrome.

> **When do you need `crw-server`?** Only if you want a REST API endpoint. The Python SDK (`CrwClient()`) and MCP binary (`crw-mcp`) both run a self-contained engine — no server required.

---

## SDKs

### Python

```bash
pip install crw
```

```python
from crw import CrwClient

# Cloud (fastcrw.com — includes web search):
client = CrwClient(api_url="https://fastcrw.com/api", api_key="YOUR_API_KEY")
# Local (embedded, no server needed):
# client = CrwClient()

# Scrape
result = client.scrape("https://example.com", formats=["markdown", "links"])
print(result["markdown"])

# Crawl (blocks until complete)
pages = client.crawl("https://docs.example.com", max_depth=2, max_pages=50)

# Map
urls = client.map("https://example.com")

# Search (cloud only)
results = client.search("AI news", limit=10, sources=["web", "news"])
```

> **Requires:** Python 3.9+. Local mode auto-downloads the `crw-mcp` binary on first use — no manual setup.

### Community SDKs

- [`crewai-crw`](https://pypi.org/project/crewai-crw/) — CRW scraping tools for CrewAI agents
- [`langchain-crw`](https://pypi.org/project/langchain-crw/) — CRW document loader for LangChain

> **Node.js:** No official SDK yet — use the REST API directly or `npx crw-mcp` for MCP. [SDK examples →](https://docs.fastcrw.com/sdk-examples/)

---

## Integrations

**Frameworks:** [CrewAI](https://pypi.org/project/crewai-crw/) · [LangChain](https://pypi.org/project/langchain-crw/) · [Agno](https://github.com/agno-agi/agno/pull/7183) · [Dify](https://github.com/langgenius/dify)

**Platforms:** [n8n](https://fastcrw.com/blog/n8n-web-scraping-crw) · [Flowise](https://github.com/FlowiseAI/Flowise/pull/6066)

Missing your favorite tool? [Open an issue →](https://github.com/us/crw/issues) · [All integrations →](https://docs.fastcrw.com/integrations/)

---

## LLM Structured Extraction

Send a JSON schema, get validated structured data back using LLM function calling. [Full extraction docs →](https://docs.fastcrw.com/extract/)

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

Configure the LLM provider:

```toml
[extraction.llm]
provider = "anthropic"        # "anthropic" or "openai"
api_key = "sk-..."            # or CRW_EXTRACTION__LLM__API_KEY env var
model = "claude-sonnet-4-20250514"
```

---

## JS Rendering

CRW auto-detects SPAs and renders them via a headless browser. [Full JS rendering docs →](https://docs.fastcrw.com/js-rendering/)

```bash
crw-server setup   # downloads LightPanda, creates config.local.toml
```

| Renderer | Protocol | Best for |
|----------|----------|----------|
| LightPanda | CDP over WebSocket | Low-resource environments (default); simple sites |
| Chrome | CDP over WebSocket | Modern React/Vite/Next SPAs; recommended for production |
| Playwright | CDP over WebSocket | Full browser compatibility |

> **Renderer choice matters for SPAs.** LightPanda is fast and cheap but its
> JS runtime does not fully cover every modern bundle format. For React /
> Vite / Next sites whose content appears only after hydration, configure
> Chrome (or Playwright) alongside LightPanda — CRW will fall back to Chrome
> automatically when LightPanda returns a loading placeholder. Leaving
> LightPanda as the *only* renderer may silently return
> `"Loading..."`-style shell content for these sites.

With Docker Compose, LightPanda runs as a sidecar automatically:

```bash
docker compose up
```

---

## CLI

Scrape any URL from your terminal — no server, no config. [Full CLI docs →](https://docs.fastcrw.com/quick-start/)

```bash
crw example.com                        # markdown to stdout
crw example.com --format html          # HTML output
crw example.com --format links         # extract all links
crw example.com --js                   # with JS rendering
crw example.com --css 'article'        # CSS selector
crw example.com --stealth              # stealth mode (rotate UAs)
crw example.com -o page.md             # write to file
```

---

## Self-Hosting

Once [installed](#api-server-crw-server--firecrawl-compatible-rest-api), start the server and optionally enable JS rendering:

```bash
crw-server                    # start REST API on :3000
crw-server setup              # optional: downloads LightPanda for JS rendering
docker compose up             # alternative: Docker with LightPanda sidecar
```

See the [self-hosting guide](https://docs.fastcrw.com/#self-hosting) for production hardening, auth, reverse proxy, and resource tuning.

---

## Open Source vs Cloud

| | Self-hosted (free) | [fastcrw.com](https://fastcrw.com) Cloud |
|---|---|---|
| Core scraping | ✅ | ✅ |
| JS rendering | ✅ (LightPanda/Chrome) | ✅ |
| Web search | ❌ | ✅ |
| Global proxy network | ❌ | ✅ |
| Dashboard | ❌ | ✅ |
| Commercial use without open-sourcing | Requires AGPL compliance | ✅ Included |
| Cost | $0 | From $13/mo |

> [**Sign up free →**](https://fastcrw.com) — **500 free credits**, no credit card required.

---

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

| Crate | Description | |
|-------|-------------|-|
| [`crw-core`](crates/crw-core) | Core types, config, and error handling | [![crates.io](https://img.shields.io/crates/v/crw-core.svg)](https://crates.io/crates/crw-core) |
| [`crw-renderer`](crates/crw-renderer) | HTTP + CDP browser rendering engine | [![crates.io](https://img.shields.io/crates/v/crw-renderer.svg)](https://crates.io/crates/crw-renderer) |
| [`crw-extract`](crates/crw-extract) | HTML → markdown/plaintext extraction | [![crates.io](https://img.shields.io/crates/v/crw-extract.svg)](https://crates.io/crates/crw-extract) |
| [`crw-crawl`](crates/crw-crawl) | Async BFS crawler with robots.txt & sitemap | [![crates.io](https://img.shields.io/crates/v/crw-crawl.svg)](https://crates.io/crates/crw-crawl) |
| [`crw-server`](crates/crw-server) | Axum API server (Firecrawl-compatible) | [![crates.io](https://img.shields.io/crates/v/crw-server.svg)](https://crates.io/crates/crw-server) |
| [`crw-mcp`](crates/crw-mcp) | MCP stdio server (embedded + proxy mode) | [![crates.io](https://img.shields.io/crates/v/crw-mcp.svg)](https://crates.io/crates/crw-mcp) |
| [`crw-cli`](crates/crw-cli) | Standalone CLI (`crw` binary, no server) | [![crates.io](https://img.shields.io/crates/v/crw-cli.svg)](https://crates.io/crates/crw-cli) |

[Full architecture docs →](https://docs.fastcrw.com/architecture/)

---

## Configuration

Layered TOML config with environment variable overrides:

1. `config.default.toml` — built-in defaults
2. `config.local.toml` — local overrides (or `CRW_CONFIG=myconfig`)
3. Environment variables — `CRW_` prefix, `__` separator (e.g. `CRW_SERVER__PORT=8080`)

```toml
[server]
host = "0.0.0.0"
port = 3000
rate_limit_rps = 10

[renderer]
mode = "auto"  # auto | lightpanda | playwright | chrome | none

[crawler]
max_concurrency = 10
requests_per_second = 10.0
respect_robots_txt = true

[auth]
# api_keys = ["fc-key-1234"]
```

See [full configuration reference](https://docs.fastcrw.com/#configuration).

---

## Security

- **SSRF protection** — blocks loopback, private IPs, cloud metadata (`169.254.x.x`), IPv6 mapped addresses, and non-HTTP schemes (`file://`, `data:`)
- **Auth** — optional Bearer token with constant-time comparison
- **robots.txt** — RFC 9309 compliant with wildcard patterns
- **Rate limiting** — token-bucket algorithm, returns 429 with `error_code`
- **Resource limits** — max body 1 MB, max crawl depth 10, max pages 1000

[Full security docs →](https://docs.fastcrw.com/self-hosting-hardening/)

---

## Resources

- [Documentation](https://docs.fastcrw.com)
- [API Reference](https://docs.fastcrw.com/#rest-api)
- [MCP Setup Guide](https://docs.fastcrw.com/#mcp)
- [Playground](https://docs.fastcrw.com/playground/)
- [Changelog](CHANGELOG.md)

---

## Contributing

Contributions are welcome! Please open an issue or submit a pull request.

1. Fork the repository
2. Install pre-commit hooks: `make hooks`
3. Create your feature branch (`git checkout -b feat/my-feature`)
4. Commit your changes (`git commit -m 'feat: add my feature'`)
5. Push to the branch (`git push origin feat/my-feature`)
6. Open a Pull Request

The pre-commit hook runs the same checks as CI (`cargo fmt`, `cargo clippy`, `cargo test`). Run manually with `make check`.

### Contributors

<a href="https://github.com/us/crw/graphs/contributors">
  <img alt="contributors" src="https://contrib.rocks/image?repo=us/crw"/>
</a>

---

## License

CRW is open-source under [AGPL-3.0](LICENSE). For a managed version without AGPL obligations, see [fastcrw.com](https://fastcrw.com).

---

## Get Started

- **Self-host free:** `curl -fsSL https://raw.githubusercontent.com/us/crw/main/install.sh | sh` — works in 30 seconds
- **Cloud:** [**Sign up free →**](https://fastcrw.com) — **500 free credits**, no credit card required
- **Questions?** [Join our Discord](https://discord.gg/kkFh2SC8)

---

**It is the sole responsibility of end users to respect websites' policies when scraping.** Users are advised to adhere to applicable privacy policies and terms of use. By default, CRW respects `robots.txt` directives.

<p align="right">
  <a href="#readme-top">↑ Back to Top ↑</a>
</p>
