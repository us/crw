<div class="page-intro">
  <div class="page-kicker">Get Started</div>
  <h1>API Overview</h1>
  <p class="page-subtitle">Use this page to pick the right CRW route quickly. The endpoint pages are the real usage guides; this page is the shortest map from your task to the right API surface.</p>
  <div class="page-capabilities">
    <div class="page-capability"><strong>Hosted base URL:</strong> <code>https://fastcrw.com/api</code></div>
    <div class="page-capability"><strong>Self-host default:</strong> <code>http://localhost:3000</code></div>
    <div class="page-capability"><strong>Best first route:</strong> <code>POST /v1/scrape</code></div>
  </div>
  <div class="page-actions">
    <a class="page-btn primary" href="#scraping">Open Scrape</a>
    <a class="page-btn secondary" href="#quick-start">Open Quick Start</a>
  </div>
</div>

## Authentication

- Hosted API: always send `Authorization: Bearer YOUR_API_KEY`
- Self-hosted API: auth is only required when `auth.api_keys` is configured
- `/health` is always public

## Pick the right endpoint

:::cards
::card{icon="code" title="Scrape" href="#scraping" description="One known URL in, clean content out. This is the default starting point."}
::card{icon="search" title="Search" href="#search" description="Hosted discovery-first workflow when you do not know the URLs yet."}
::card{icon="map" title="Map" href="#map" description="Discover URLs first without paying for full scraping."}
::card{icon="globe" title="Crawl" href="#crawling" description="Run an async multi-page job when one page is not enough."}
::card{icon="zap" title="Extract" href="#extract" description="Return structured JSON when downstream systems want fields, not prose."}
::card{icon="plug" title="MCP" href="#mcp" description="Expose CRW as tools to Claude, Codex, Cursor, and other MCP hosts."}
:::

## Route summary

| Route | Use this when | Notes |
|---|---|---|
| `POST /v1/scrape` | You already know the page URL | Core hosted and self-hosted route |
| `POST /v1/search` | You need discovery across the web | Hosted/cloud path |
| `POST /v1/map` | You want discovery before content extraction | Returns links only |
| `POST /v1/crawl` | You need many pages, asynchronously | Poll with `GET /v1/crawl/{id}` |
| `GET /v1/crawl/{id}` | You need crawl progress and results | Returns status plus completed data |
| `DELETE /v1/crawl/{id}` | You want to cancel an active crawl | Hosted and self-hosted |
| `POST /mcp` | You are using MCP over HTTP | Prefer the [MCP page](#mcp) for setup |

## Start with this request

```bash
curl -X POST https://fastcrw.com/api/v1/scrape \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"url":"https://example.com","formats":["markdown"]}'
```

If that request works, continue into [Scrape](#scraping) and only then branch into search, map, crawl, or extraction.

## Shared reference pages

- [Response Shapes](#response-shapes) for common envelopes
- [Output Formats](#output-formats) for `markdown`, `html`, `rawHtml`, `plainText`, `links`, and `json`
- [Error Codes](#error-codes) for failure handling
- [Rate Limits](#rate-limits) and [Credit Costs](#credit-costs) for hosted usage planning

## What to read next

- [Quick Start](#quick-start)
- [Scrape](#scraping)
- [Search](#search)
- [Map](#map)
- [Crawl](#crawling)
