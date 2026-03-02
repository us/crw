# CRW

Lightweight, Firecrawl-compatible web scraper. Single binary, ~3MB idle RAM, optional JS rendering via LightPanda sidecar.

**API-compatible with [Firecrawl](https://firecrawl.dev)** — drop-in replacement for self-hosted deployments.

## Quick Start

```bash
# Build
cargo build --release --bin crw-server

# Run
./target/release/crw-server

# Scrape a page
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

## Features

- **4 API endpoints** — `/v1/scrape`, `/v1/crawl`, `/v1/crawl/:id`, `/v1/map`
- **Multiple output formats** — markdown, HTML, plain text, links
- **JS rendering** — auto-detect SPAs, render via LightPanda/Playwright/Chrome (CDP)
- **BFS crawler** — async crawl with rate limiting, robots.txt, sitemap support
- **LLM extraction** — structured JSON output via Claude or OpenAI
- **MCP server** — use as a tool in Claude Code / Claude Desktop
- **Auth** — optional Bearer token authentication
- **Docker ready** — multi-stage Dockerfile + docker-compose with LightPanda

## API Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| `POST` | `/v1/scrape` | Scrape a single URL |
| `POST` | `/v1/crawl` | Start async crawl (returns job ID) |
| `GET` | `/v1/crawl/:id` | Check crawl status / get results |
| `POST` | `/v1/map` | Discover URLs on a site |
| `GET` | `/health` | Health check (no auth) |

## MCP Server (Claude Code / Desktop)

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

Tools: `crw_scrape`, `crw_crawl`, `crw_check_crawl_status`, `crw_map`

## Docker

```bash
docker compose up
```

## Performance

| Metric | Value |
|--------|-------|
| Idle RAM | 3.3 MB (server) + 3.3 MB (LightPanda) |
| HTTP scrape | ~30ms avg |
| JS scrape | ~520ms avg |
| Cold start | ~85ms |

## Documentation

Full documentation: **[docs/index.md](docs/index.md)**

- [Installation](docs/index.md#installation)
- [Configuration](docs/index.md#configuration)
- [API Reference](docs/index.md#api-reference)
- [MCP Server](docs/index.md#mcp-server)
- [JS Rendering](docs/index.md#js-rendering)
- [Architecture](docs/index.md#architecture)

## License

MIT
