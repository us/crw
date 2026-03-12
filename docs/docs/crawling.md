# Web Crawling API

CRW crawls entire websites using breadth-first search (BFS) — a self-hosted alternative to Firecrawl's crawl endpoint. Crawl jobs are asynchronous — you start a crawl, get a job ID, and poll for results. Includes robots.txt compliance, sitemap discovery, rate limiting, and concurrent page processing.

## Start a Crawl

```
POST /v1/crawl
```

```json
{
  "url": "https://docs.example.com",
  "maxDepth": 2,
  "maxPages": 100,
  "formats": ["markdown"],
  "onlyMainContent": true,
  "jsonSchema": null
}
```

| Field | Type | Default | Max | Description |
|-------|------|---------|-----|-------------|
| `url` | string | required | — | Starting URL |
| `maxDepth` | int | 2 | 10 | Maximum link depth from start URL |
| `maxPages` | int | 100 | 1000 | Maximum pages to crawl |
| `formats` | string[] | `["markdown"]` | — | Output formats per page |
| `onlyMainContent` | bool | `true` | — | Extract only main content |
| `jsonSchema` | object | — | — | JSON schema for LLM extraction |

Response:

```json
{
  "success": true,
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "url": "http://localhost:3000/v1/crawl/550e8400-e29b-41d4-a716-446655440000"
}
```

## Check Status

```
GET /v1/crawl/{id}
```

Returns the crawl state, pages completed, and results collected so far.

## Cancel a Crawl

```
DELETE /v1/crawl/{id}
```

Cancels a running crawl job. The crawl task is aborted immediately via its `AbortHandle`. Returns an error if the job doesn't exist or has already completed.

```json
{
  "success": true,
  "message": "Crawl job cancelled"
}
```

## How BFS Crawling Works

1. Start URL is added to the queue at depth 0
2. For each URL in the queue, crw scrapes the page and extracts all links
3. New links within the same origin are added to the queue at depth + 1
4. Continues until `maxDepth` or `maxPages` is reached, or no more URLs remain

### Concurrency & Rate Limiting

- **Semaphore** — Limits concurrent requests (default: 10, set via `crawler.max_concurrency`)
- **Rate limiter** — Enforces minimum interval between requests (default: 10 RPS, set via `crawler.requests_per_second`)

### Domain Restriction

Crawling stays within the same origin (scheme + host). Links to external domains are ignored.

### URL Normalization

Before adding to the visited set, URLs are normalized:
- Fragment (`#...`) is removed
- Trailing slash is normalized
- Scheme and host are lowercased

## Robots.txt

crw respects `robots.txt` by default (`crawler.respect_robots_txt = true`).

- Fetches and parses `robots.txt` before crawling
- Checks `User-agent: crw` first, then `User-agent: *`
- Supports `Allow`, `Disallow`, and `Sitemap` directives
- Supports wildcard `*` and end anchor `$` in patterns
- Uses RFC 9309 specificity matching: longest pattern wins; on tie, `Allow` wins

## Site Mapping

The `POST /v1/map` endpoint discovers URLs without scraping content:

```json
{
  "url": "https://example.com",
  "maxDepth": 2,
  "useSitemap": true
}
```

When `useSitemap` is true, crw checks:
1. `Sitemap:` directives in `robots.txt`
2. `{origin}/sitemap.xml` as a fallback
3. Supports both `<urlset>` and sitemap index formats

Maximum discovered URLs: 5000.

## Job Lifecycle

Crawl jobs have a configurable TTL (`crawler.job_ttl_secs`, default: 3600). A background task cleans up expired jobs every 60 seconds.
