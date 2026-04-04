# Web Crawling API

CRW crawls entire websites using breadth-first search (BFS) — a self-hosted alternative to Firecrawl's crawl endpoint. Crawl jobs are asynchronous — you start a crawl, get a job ID, and poll for results. Includes robots.txt compliance, sitemap discovery, rate limiting, and concurrent page processing.

Use `crawl` when you need multiple pages instead of a single response payload. It is the right tool for:

- documentation sections,
- knowledge-base ingestion,
- internal search refreshes,
- and recursive collection jobs that start from one known URL.

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
| `limit` | int | — | — | Alias for `maxPages` (Firecrawl compatibility) |
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

## Start, Then Poll

The crawl API is asynchronous by design.

1. `POST /v1/crawl` starts the job.
2. The API returns a crawl id.
3. `GET /v1/crawl/:id` returns progress and newly available results.
4. Continue polling until the status becomes `completed` or a terminal error is returned.

```bash
curl http://localhost:3000/v1/crawl/CRAWL_ID
```

That flow is easy to drive from shell scripts, job runners, background workers, and dashboards.

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

## A Practical Evaluation Loop

The safest way to evaluate a new site is:

1. run `map` first to understand the reachable section,
2. launch a crawl with a low page cap,
3. inspect the resulting markdown or extraction output,
4. then widen the scope only after the first batch looks good.

:::tip
Start small. A crawl with `maxPages: 5` is much easier to inspect than a crawl with `maxPages: 500`. That sequence saves credits and helps you catch bad starting URLs early.
:::

## Job Lifecycle

Crawl jobs have a configurable TTL (`crawler.job_ttl_secs`, default: 3600). A background task cleans up expired jobs every 60 seconds.

## Credit and Retry Behavior

Available on [fastcrw.com](https://fastcrw.com) (cloud):

- Starting a crawl consumes the initial crawl credit.
- Polling is tied to newly materialized pages.
- Transient upstream failures should be handled with retry logic rather than blind rapid polling.

:::note
If the API returns `429`, respect `Retry-After`. If the target site itself is slow or hostile, reducing crawl size usually gives you a clearer signal than hammering the same job harder.
:::
