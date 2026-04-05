# REST API Reference — Firecrawl-Compatible

Base URL: `https://fastcrw.com/api`

All `/v1/*` and `/mcp` endpoints require authentication when API keys are configured. The `/health` endpoint is always public.

## Authentication

```
Authorization: Bearer fc-your-api-key
```

Token comparison uses constant-time equality to prevent timing attacks.

## Health Check

```
GET /health
```

```json
{
  "status": "ok",
  "version": "0.0.1",
  "renderers": {
    "http": true
  },
  "active_crawl_jobs": 0
}
```

## Scrape

```
POST /v1/scrape
```

### Request Body

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `url` | string | **yes** | — | URL to scrape (`http`/`https` only) |
| `formats` | string[] | no | `["markdown"]` | Output formats (see below) |
| `onlyMainContent` | boolean | no | `true` | Strip navigation, footer, sidebar |
| `renderJs` | boolean/null | no | `null` | `null`=auto, `true`=force JS, `false`=HTTP only |
| `waitFor` | number | no | — | Milliseconds to wait after JS rendering |
| `includeTags` | string[] | no | `[]` | CSS selectors — only include matching elements |
| `excludeTags` | string[] | no | `[]` | CSS selectors — remove matching elements |
| `headers` | object | no | `{}` | Custom HTTP request headers |
| `jsonSchema` | object | no | — | JSON Schema for LLM structured extraction |
| `cssSelector` | string | no | — | Extract only elements matching this CSS selector |
| `xpath` | string | no | — | Extract only elements matching this XPath expression |
| `chunkStrategy` | object | no | — | Split content into chunks (see [Chunking](#scraping)) |
| `query` | string | no | — | Query for chunk ranking (used with `filterMode`) |
| `filterMode` | string | no | — | Chunk ranking method: `"bm25"` or `"cosine"` |
| `topK` | number | no | `5` | Number of top-ranked chunks to return |
| `proxy` | string | no | — | Per-request proxy URL (e.g. `"http://user:pass@host:port"`) |
| `stealth` | boolean | no | — | Override global stealth mode for this request |

Snake case aliases are also accepted: `only_main_content`, `render_js`, `wait_for`, `include_tags`, `exclude_tags`, `json_schema`, `css_selector`, `chunk_strategy`, `filter_mode`, `top_k`.

**Output formats:**

| Format | Description |
|--------|-------------|
| `markdown` | Cleaned HTML converted to Markdown |
| `html` | Cleaned HTML (scripts, styles, ads removed) |
| `rawHtml` | Original HTML as-is |
| `plainText` | Text content only, no markup |
| `links` | Array of all links found on the page |
| `json` | LLM-extracted structured data (requires `jsonSchema` + LLM config) |

### Response Body

```json
{
  "success": true,
  "data": {
    "markdown": "string or null",
    "html": "string or null",
    "rawHtml": "string or null",
    "plainText": "string or null",
    "links": ["string"],
    "json": {},
    "chunks": ["string or null"],
    "metadata": {
      "title": "string",
      "description": "string",
      "ogTitle": "string",
      "ogDescription": "string",
      "ogImage": "string",
      "canonicalUrl": "string",
      "sourceURL": "string",
      "language": "string",
      "statusCode": 200,
      "renderedWith": "string",
      "elapsedMs": 32
    }
  }
}
```

Only requested formats are populated. Others are `null`. `chunks` is populated when `chunkStrategy` is provided.

### Examples

**Basic scrape:**

```bash
curl -X POST https://fastcrw.com/api/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'
```

**Multiple formats:**

```bash
curl -X POST https://fastcrw.com/api/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com",
    "formats": ["markdown", "html", "links"]
  }'
```

**Force JS rendering:**

```bash
curl -X POST https://fastcrw.com/api/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://spa-app.example.com",
    "renderJs": true,
    "waitFor": 2000
  }'
```

## Chunking

Split scraped content into chunks for RAG pipelines. Set `chunkStrategy` to enable, optionally combined with `query` + `filterMode` to rank the results.

### Chunk strategies

**Topic** — split on Markdown headings (`##`, `###`):

```json
{
  "url": "https://en.wikipedia.org/wiki/Rust_(programming_language)",
  "formats": ["markdown"],
  "chunkStrategy": { "type": "topic" }
}
```

**Sentence** — split on sentence boundaries, merge until `maxChars`:

```json
{
  "chunkStrategy": { "type": "sentence", "maxChars": 500 }
}
```

**Regex** — split on a custom pattern:

```json
{
  "chunkStrategy": { "type": "regex", "pattern": "\\n\\n" }
}
```

### BM25 / Cosine filtering

Rank chunks by relevance to a query and return the top K:

```json
{
  "url": "https://en.wikipedia.org/wiki/Rust_(programming_language)",
  "formats": ["markdown"],
  "chunkStrategy": { "type": "topic" },
  "query": "memory safety ownership borrow checker",
  "filterMode": "bm25",
  "topK": 5
}
```

| `filterMode` | Algorithm |
|---|---|
| `bm25` | BM25 keyword relevance (fast, no dependencies) |
| `cosine` | TF-IDF cosine similarity (semantic overlap) |

Chunks are returned in `data.chunks` (array of strings), ordered by relevance score.

## CSS Selector & XPath

Extract a specific part of the page before converting to Markdown.

**CSS selector:**

```json
{
  "url": "https://news.ycombinator.com",
  "formats": ["markdown"],
  "cssSelector": "td.title",
  "onlyMainContent": false
}
```

**XPath:**

```json
{
  "url": "https://news.ycombinator.com",
  "formats": ["markdown"],
  "xpath": "//span[@class='titleline']/a",
  "onlyMainContent": false
}
```

When a selector is provided, only the matching HTML is converted to Markdown. `onlyMainContent` and readability filtering are skipped.

## Stealth Mode & Proxy

### Stealth mode

Inject browser-like headers to reduce bot-detection fingerprinting. Enable per-request:

```json
{
  "url": "https://example.com",
  "stealth": true
}
```

When `stealth` is `true`, CRW:
- Rotates User-Agent from a built-in pool of realistic Chrome, Firefox, and Safari strings
- Injects `Accept`, `Accept-Language`, `Accept-Encoding`, `Sec-Ch-Ua`, `Sec-Ch-Ua-Mobile`, `Sec-Ch-Ua-Platform`, `Sec-Fetch-*`, `Priority`, and `Upgrade-Insecure-Requests` headers

Enable globally in config:

```toml
[crawler]
stealth = true
```

### Per-request proxy

```json
{
  "url": "https://example.com",
  "proxy": "http://user:pass@proxy-host:8080"
}
```

The global proxy (`crawler.proxy` in config) is used when no per-request proxy is set.

## Crawl

### Start a Crawl

```
POST /v1/crawl
```

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `url` | string | **yes** | — | Starting URL |
| `maxDepth` | number | no | `2` | Maximum link-follow depth |
| `maxPages` | number | no | `100` | Maximum pages to scrape |
| `formats` | string[] | no | `["markdown"]` | Output formats for each page |
| `onlyMainContent` | boolean | no | `true` | Strip boilerplate |

Response:

```json
{
  "success": true,
  "id": "a4c03342-ab36-4df6-9e15-7ecffc9f8b3a"
}
```

### Check Crawl Status

```
GET /v1/crawl/{id}
```

```json
{
  "status": "completed",
  "total": 1,
  "completed": 1,
  "data": [
    {
      "markdown": "# Example Domain\n...",
      "metadata": {
        "title": "Example Domain",
        "sourceURL": "https://example.com",
        "statusCode": 200,
        "elapsedMs": 12
      }
    }
  ]
}
```

| Status | Description |
|--------|-------------|
| `scraping` | Crawl is in progress |
| `completed` | All pages scraped |
| `failed` | Fatal error |

Completed crawl jobs are automatically cleaned up after the configured TTL (default: 1 hour).

## Map

```
POST /v1/map
```

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `url` | string | **yes** | — | URL to discover links from |
| `maxDepth` | number | no | `2` | Maximum discovery depth |
| `useSitemap` | boolean | no | `true` | Also read sitemap.xml |

```json
{
  "success": true,
  "data": {
    "links": ["https://example.com", "https://example.com/about"]
  }
}
```

## Search (Cloud Only)

> Available exclusively on [fastcrw.com](https://fastcrw.com). Not included in the self-hosted binary.

```
POST https://fastcrw.com/api/v1/search
```

Search the web and optionally scrape the results in one operation.

### Request Body

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `query` | string | **yes** | — | Search query |
| `limit` | number | no | `5` | Max results (1–20) |
| `lang` | string | no | — | Language code (e.g. `"en"`, `"tr"`) |
| `tbs` | string | no | — | Time filter: `qdr:h`, `qdr:d`, `qdr:w`, `qdr:m`, `qdr:y` |
| `sources` | string[] | no | — | Result types: `"web"`, `"news"`, `"images"` |
| `categories` | string[] | no | — | Category filters: `"github"`, `"research"`, `"pdf"` |
| `scrapeOptions` | object | no | — | Scrape each result URL (e.g. `{ "formats": ["markdown"] }`) |

**`scrapeOptions` fields:**

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `formats` | string[] | **yes** | — | Output formats: `"markdown"`, `"html"`, `"rawHtml"`, `"links"` |
| `onlyMainContent` | boolean | no | `true` | Extract primary content area only |

### Response (flat — no sources)

```json
{
  "success": true,
  "data": [
    {
      "url": "https://example.com/result",
      "title": "Result Title",
      "description": "Snippet from the search engine.",
      "score": 8.5,
      "position": 1,
      "category": "general",
      "markdown": "# Page content (when scrapeOptions is set)"
    }
  ]
}
```

### Response (grouped — with sources)

When `sources` is specified, results are grouped by type:

```json
{
  "success": true,
  "data": {
    "web": [{ "url": "...", "title": "...", "description": "..." }],
    "news": [{ "url": "...", "title": "...", "description": "..." }],
    "images": [{ "url": "...", "title": "...", "description": "...", "imageUrl": "...", "thumbnailUrl": "...", "position": 1 }]
  }
}
```

### Credit Cost

| Operation | Credits |
|-----------|---------|
| Search | 1 |
| Search + scrape | 1 + 1 per scraped result (failed scrapes refunded) |

### Example

```bash
curl -X POST https://fastcrw.com/api/v1/search \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "query": "web scraping tools 2026",
    "limit": 5,
    "scrapeOptions": { "formats": ["markdown"] }
  }'
```

## MCP (Streamable HTTP)

```
POST /mcp
```

Accepts JSON-RPC 2.0 requests for MCP protocol over HTTP. See [MCP Server](#mcp) for details.

## Cancel a Crawl

```
DELETE /v1/crawl/{id}
```

Cancels a running crawl job. Returns an error if the job doesn't exist or has already completed.

```json
{
  "success": true,
  "message": "Crawl job cancelled"
}
```

## Error Responses

```json
{
  "success": false,
  "error": "Human-readable error message",
  "error_code": "machine_readable_code"
}
```

All error responses include a machine-readable `error_code` field. Common codes: `invalid_url`, `invalid_request`, `http_error`, `rate_limited`, `not_found`, `renderer_error`, `timeout`, `method_not_allowed`, `internal_error`.

| Status | When |
|--------|------|
| `200` | Successful request |
| `400` | Invalid URL, missing required fields, non-http(s) scheme |
| `401` | Missing or invalid Bearer token |
| `404` | Crawl job ID doesn't exist |
| `405` | Wrong HTTP method for endpoint |
| `422` | LLM extraction failed |
| `429` | Rate limit exceeded |
| `502` | Target website returned an error |
| `504` | Request timed out |
| `500` | Unexpected server error |

## Limits

| Limit | Value |
|-------|-------|
| Request body | 1 MB |
| HTTP response | 10 MB |
| Max crawl depth | 10 |
| Max crawl pages | 1,000 |
| Max discovered URLs | 5,000 |
| Request timeout | Configurable (default: 120s) |
| URL schemes | `http`, `https` only |
