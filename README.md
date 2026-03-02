# CRW

**Lightweight, Firecrawl-compatible web scraper and crawler for AI**

CRW is a self-hosted web scraper and web crawler built in Rust — a fast, lightweight Firecrawl alternative designed for LLM extraction, RAG pipelines, and AI agents. It ships as a single binary with ~3 MB idle RAM, built-in MCP server support for Claude, and structured data extraction via Anthropic and OpenAI. Drop-in compatible with Firecrawl's API.

**English** | [中文](README.zh-CN.md)

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE-MIT)
[![Rust](https://img.shields.io/badge/Rust-2024_edition-orange.svg)](https://www.rust-lang.org/)

## Why CRW?

CRW gives you Firecrawl's API with a fraction of the resource usage. No runtime dependencies, no Redis, no Node.js — just a single binary you can deploy anywhere.

| | CRW | Firecrawl |
|---|---|---|
| **Coverage (1K URLs)** | **91.5%** | 77.2% |
| **Avg Latency** | **833ms** | 4,600ms |
| **P50 Latency** | **446ms** | — |
| **Noise Rejection** | **89.1%** | — |
| **Idle RAM** | 6.6 MB | ~500 MB+ |
| **Cold start** | 85 ms | seconds |
| **HTTP scrape** | ~30 ms | ~200 ms+ |
| **Binary size** | ~8 MB | Node.js runtime |
| **Cost / 1K scrapes** | **$0** (self-hosted) | $0.83–5.33 |
| **Dependencies** | single binary | Node + Redis |
| **License** | MIT | AGPL |

Benchmark: [Firecrawl scrape-content-dataset-v1](https://huggingface.co/datasets/firecrawl/scrape-content-dataset-v1) — 1,000 real-world URLs with JS rendering enabled.

## Features

- **🔌 Firecrawl-compatible API** — same endpoints, same request/response format, drop-in replacement
- **📄 6 output formats** — markdown, HTML, cleaned HTML, raw HTML, plain text, links, structured JSON
- **🤖 LLM structured extraction** — send a JSON schema, get validated structured data back (Anthropic tool_use + OpenAI function calling)
- **🌐 JS rendering** — auto-detect SPAs with shell heuristics, render via LightPanda, Playwright, or Chrome (CDP)
- **🕷️ BFS crawler** — async crawl with rate limiting, robots.txt, sitemap support, concurrent jobs
- **🔧 MCP server** — built-in stdio + HTTP transport for Claude Code and Claude Desktop
- **🔒 Auth** — optional Bearer token with constant-time comparison
- **🐳 Docker ready** — multi-stage build with LightPanda sidecar

## Quick Start

**Build from source:**

```bash
cargo build --release --bin crw-server
./target/release/crw-server
```

**Docker:**

```bash
docker compose up
```

**Scrape a page:**

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
| `POST` | `/v1/map` | Discover all URLs on a site |
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

CRW works as an MCP tool server for Claude Code and Claude Desktop with two transports.

**HTTP transport (recommended):**

```bash
claude mcp add --transport http crw http://localhost:3000/mcp
```

**Stdio transport:**

```bash
cargo build --release --bin crw-mcp
```

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

**Tools:** `crw_scrape`, `crw_crawl`, `crw_check_crawl_status`, `crw_map`

## JS Rendering

CRW auto-detects SPAs by analyzing the initial HTML response for shell heuristics (empty body, framework markers). When a SPA is detected, it renders the page via a headless browser.

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

[renderer]
mode = "auto"  # auto | lightpanda | playwright | chrome | none

[crawler]
max_concurrency = 10
requests_per_second = 10.0
respect_robots_txt = true

[auth]
# api_keys = ["fc-key-1234"]
```

See [full configuration reference](docs/index.md#configuration) for all options.

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

**LangChain document loader pattern:**

```python
import requests

def load_documents(urls):
    documents = []
    for url in urls:
        resp = requests.post("http://localhost:3000/v1/scrape", json={
            "url": url,
            "formats": ["markdown"]
        })
        data = resp.json()["data"]
        documents.append({
            "page_content": data["markdown"],
            "metadata": data["metadata"]
        })
    return documents
```

## Docker

```bash
docker compose up
```

This starts CRW on port `3000` with LightPanda as a JS rendering sidecar on port `9222`. CRW auto-connects to LightPanda for SPA rendering.

## Benchmark

Tested on [Firecrawl's scrape-content-dataset-v1](https://huggingface.co/datasets/firecrawl/scrape-content-dataset-v1) (1,000 real-world URLs, JS rendering enabled):

| | CRW | Firecrawl v2.5 |
|---|---|---|
| **Coverage** | **91.5%** | 77.2% |
| **Avg Latency** | **833ms** | 4,600ms |
| **P50 Latency** | **446ms** | — |
| **Noise Rejection** | **89.1%** | — |
| **Cost / 1,000 scrapes** | **$0** (self-hosted) | $0.83–5.33 |
| **Idle RAM** | **6.6 MB** | ~500 MB+ |

Run the benchmark yourself:

```bash
pip install datasets aiohttp
python3 bench/run_bench.py
```

## Documentation

Full documentation: **[docs/index.md](docs/index.md)**

- [Getting Started](docs/index.md#installation)
- [Configuration](docs/index.md#configuration)
- [API Reference](docs/index.md#api-reference)
- [MCP Server](docs/index.md#mcp-server)
- [JS Rendering](docs/index.md#js-rendering)
- [Architecture](docs/index.md#architecture)

## Contributing

Contributions are welcome! Please open an issue or submit a pull request.

1. Fork the repository
2. Create your feature branch (`git checkout -b feat/my-feature`)
3. Commit your changes (`git commit -m 'feat: add my feature'`)
4. Push to the branch (`git push origin feat/my-feature`)
5. Open a Pull Request

## License

[MIT](LICENSE-MIT)
