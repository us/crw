<div class="page-intro">
  <div class="page-kicker">Get Started</div>
  <h1>CRW Docs</h1>
  <p class="page-subtitle">Turn websites into usable data with one API. Start with a single <code>scrape</code> request, then move into <code>search</code>, <code>map</code>, <code>crawl</code>, <code>extract</code>, or MCP only when your workflow actually needs them.</p>
  <div class="page-capabilities">
    <div class="page-capability"><strong>Fastest first win:</strong> one URL, one markdown response</div>
    <div class="page-capability"><strong>Works for:</strong> agents, ETL, RAG, structured extraction</div>
    <div class="page-capability"><strong>Deploy:</strong> cloud first, self-host when ready</div>
  </div>
  <div class="page-actions">
    <a class="page-btn primary" href="#quick-start">Make your first request</a>
    <a class="page-btn secondary" href="#self-hosting">Self-host CRW</a>
  </div>
</div>

<div class="playground-panel">
  <div class="playground-kicker">30-second example</div>
  <div class="playground-title">The shortest path to a successful response</div>
  <div class="playground-copy">If this request works, you already understand the core CRW model: known URL in, clean content out. Everything else in the docs builds on that.</div>
</div>

```bash
curl -X POST https://fastcrw.com/api/v1/scrape \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com",
    "formats": ["markdown"]
  }'
```

```json
{
  "success": true,
  "data": {
    "markdown": "# Example Domain\n\nThis domain is for use in illustrative examples...",
    "metadata": {
      "title": "Example Domain",
      "sourceURL": "https://example.com",
      "statusCode": 200,
      "elapsedMs": 32
    }
  }
}
```

## Start here

:::cards
::card{icon="code" title="Scrape a page" href="#scraping" description="Use one URL and get markdown, HTML, links, or JSON back."}
::card{icon="search" title="Search the web" href="#search" description="Find URLs first, then scrape only the results you care about."}
::card{icon="plug" title="Add MCP tools" href="#mcp" description="Give Claude, Cursor, Codex, and other hosts live web access."}
:::

## Choose your path

:::cards
::card{icon="rocket" title="Cloud API" href="#quick-start" description="The fastest first run: get a key, copy one request, and move."}
::card{icon="plug" title="MCP" href="#mcp" description="Best when your agent runtime already expects MCP tools."}
::card{icon="box" title="Self-host" href="#self-hosting" description="Best when you want your own infrastructure, auth, and deployment controls."}
:::

## Why teams switch to CRW

CRW is meant to feel easy on day one without closing off the more serious use cases:

- one API surface for single-page scrape, discovery, bounded crawl, and extraction,
- Firecrawl-compatible request shapes where they matter,
- low-ops self-hosting when you need infra control,
- and a built-in MCP server for agent workflows.

## Benchmarks

Tested against [Firecrawl scrape-content-dataset-v1](https://huggingface.co/datasets/firecrawl/scrape-content-dataset-v1) on 1,000 real-world URLs:

| Metric | CRW | Firecrawl | Crawl4AI |
|---|---|---|---|
| **Coverage (1K URLs)** | **92.0%** | 77.2% | — |
| **Avg Latency** | **833ms** | 4,600ms | — |
| **Idle RAM** | **6.6 MB** | ~500 MB+ | — |
| **Cold start** | **85 ms** | 30–60 s | — |
| **Dependencies** | single binary | Node + Redis + PG + RabbitMQ | Python + Playwright |

## What to read next

- [Quick Start](#quick-start) for the fastest first request
- [API Overview](#rest-api) for the endpoint map
- [Scrape](#scraping) for the canonical single-page flow
- [Authentication](#authentication) for key handling and self-host auth
