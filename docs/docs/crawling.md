<div class="page-intro">
  <div class="page-kicker">More API</div>
  <h1>Crawl</h1>
  <p class="page-subtitle">Recursively crawl a site when one page is not enough. CRW crawl is asynchronous by design: start a job, poll it, and widen scope only after the first batch looks correct.</p>
  <div class="page-capabilities">
    <div class="page-capability"><strong>Best for:</strong> many pages from one site</div>
    <div class="page-capability"><strong>Model:</strong> async start and poll</div>
    <div class="page-capability"><strong>Start with:</strong> a very small crawl</div>
  </div>
  <div class="page-actions">
    <a class="page-btn primary" href="#map">Use Map First</a>
    <a class="page-btn secondary" href="#scraping">View Scrape</a>
  </div>
</div>

<div class="playground-panel">
  <div class="playground-kicker">Try it in the Playground</div>
  <div class="playground-title">Validate scope before you scale up</div>
  <div class="playground-copy">Use <code>maxPages: 5</code> and <code>maxDepth: 1</code> first. If the returned batch is wrong, a larger crawl only makes the mistake more expensive.</div>
  <div class="playground-actions">
    <a class="page-btn primary" href="https://fastcrw.com/playground" target="_blank" rel="noopener">Open Playground</a>
    <a class="page-btn secondary" href="#quick-start">Open Quick Start</a>
  </div>
</div>

## Crawling a site with CRW

### /v1/crawl

```http
POST /v1/crawl
GET /v1/crawl/{id}
DELETE /v1/crawl/{id}
```

Authentication:

- Hosted: send `Authorization: Bearer YOUR_API_KEY`
- Self-hosted: only required when `auth.api_keys` is configured

### Installation

CRW crawl is also plain HTTP. You start the job with one request and check its status with another.

### Basic usage

Start with this request:

```json
{
  "url": "https://docs.example.com",
  "maxDepth": 1,
  "maxPages": 5,
  "formats": ["markdown"],
  "onlyMainContent": true
}
```

:::tabs
::tab{title="Python"}
```python
import requests
import time

start = requests.post(
    "https://fastcrw.com/api/v1/crawl",
    headers={"Authorization": "Bearer YOUR_API_KEY"},
    json={
        "url": "https://docs.example.com",
        "maxDepth": 1,
        "maxPages": 5,
        "formats": ["markdown"],
    },
)

crawl_id = start.json()["id"]
time.sleep(2)
status = requests.get(
    f"https://fastcrw.com/api/v1/crawl/{crawl_id}",
    headers={"Authorization": "Bearer YOUR_API_KEY"},
)

print(status.json()["status"])
```
::tab{title="Node.js"}
```javascript
const start = await fetch("https://fastcrw.com/api/v1/crawl", {
  method: "POST",
  headers: {
    "Authorization": "Bearer YOUR_API_KEY",
    "Content-Type": "application/json"
  },
  body: JSON.stringify({
    url: "https://docs.example.com",
    maxDepth: 1,
    maxPages: 5,
    formats: ["markdown"]
  })
});

const { id } = await start.json();
await new Promise((resolve) => setTimeout(resolve, 2000));

const status = await fetch(`https://fastcrw.com/api/v1/crawl/${id}`, {
  headers: { "Authorization": "Bearer YOUR_API_KEY" }
});

console.log((await status.json()).status);
```
::tab{title="cURL"}
```bash
curl -X POST https://fastcrw.com/api/v1/crawl \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://docs.example.com",
    "maxDepth": 1,
    "maxPages": 5,
    "formats": ["markdown"]
  }'
```
:::

### Response

Start response:

```json
{
  "success": true,
  "id": "550e8400-e29b-41d4-a716-446655440000"
}
```

Poll response:

```json
{
  "success": true,
  "status": "scraping",
  "total": 5,
  "completed": 2,
  "data": []
}
```

## Parameters

| Field | Type | Default | Description |
|---|---|---|---|
| `url` | string | required | Starting URL |
| `maxDepth` | number | `2` | Maximum depth from the start URL |
| `maxPages` | number | `100` | Maximum number of pages to crawl |
| `limit` | number | alias | Firecrawl-compatible alias for `maxPages` |
| `max_pages` | number | alias | Snake_case alias for `maxPages` |
| `formats` | string[] | `["markdown"]` | Output formats for each page |
| `onlyMainContent` | boolean | `true` | Remove boilerplate content before conversion |
| `jsonSchema` | object | -- | Optional schema for structured extraction per page |
| `renderJs` | boolean or null | `null` | `true` forces JS on every page, `false` skips JS, `null` uses auto-detect or the server's `render_js_default` |
| `waitFor` | number | -- | Milliseconds to wait after JS rendering on each page |
| `renderer` | string | `auto` | Pin every crawled page to a specific renderer: `auto`, `lightpanda`, `chrome`, or `playwright`. Non-`auto` values hard-pin (no fallback) and imply `renderJs:true` unless `renderJs:false` is set. Validation runs once at crawl start — invalid combinations return HTTP 400 before the job is queued. Per-page failures of a pinned renderer are logged and skipped, so failed pages may be missing from results — see [JS rendering](#js-rendering) for the resilience tradeoff |

## Scrape options and extraction

Crawl inherits the same content-format logic as scrape:

- Start with `formats: ["markdown"]`
- Add extraction only after the first crawl batch looks correct
- Keep `onlyMainContent: true` unless you explicitly need full-page noise

If you need to debug one problematic page, go back to [Scrape](#scraping) and validate that page in isolation first.

## Checking job status

Poll the crawl ID until you reach `completed` or `failed`:

```bash
curl -H "Authorization: Bearer YOUR_API_KEY" \
  https://fastcrw.com/api/v1/crawl/CRAWL_ID
```

Status response shape:

```json
{
  "success": true,
  "status": "scraping | completed | failed",
  "total": 12,
  "completed": 12,
  "data": [
    {
      "markdown": "# Page content",
      "metadata": {
        "sourceURL": "https://example.com/page"
      }
    }
  ],
  "error": "optional error"
}
```

## Cancellation and limits

Cancel a running job with:

```http
DELETE /v1/crawl/{id}
```

CRW crawl stays within the same origin and should be treated as a bounded, respectful site job, not an open-ended spider.

## Common production patterns

- Run [Map](#map) first when you are unsure about the reachable section.
- Keep `maxPages` very low on first contact with a new site.
- Poll with backoff instead of hammering the same crawl ID.
- Use extraction only after the markdown output of the first crawl batch looks correct.

## Common mistakes

- Starting with `maxPages: 500` before validating the target
- Treating `crawl` like a synchronous route
- Assuming crawl crosses origins
- Ignoring `robots.txt` and target-side rate behavior

## When to use something else

- Use [Map](#map) when you need URL discovery before crawling
- Use [Scrape](#scraping) when you only need one known page
- Use [Search](#search) when you do not even know the site or URL set yet
