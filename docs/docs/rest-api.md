# REST API

Base URL: `http://localhost:3000`

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

Snake case aliases are also accepted: `only_main_content`, `render_js`, `wait_for`, `include_tags`, `exclude_tags`, `json_schema`.

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

Only requested formats are populated. Others are `null`.

### Examples

**Basic scrape:**

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'
```

**Multiple formats:**

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com",
    "formats": ["markdown", "html", "links"]
  }'
```

**Force JS rendering:**

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://spa-app.example.com",
    "renderJs": true,
    "waitFor": 2000
  }'
```

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
  "links": ["https://example.com", "https://example.com/about"]
}
```

## MCP (Streamable HTTP)

```
POST /mcp
```

Accepts JSON-RPC 2.0 requests for MCP protocol over HTTP. See [MCP Server](#mcp) for details.

## Error Responses

```json
{
  "success": false,
  "error": "Human-readable error message"
}
```

| Status | When |
|--------|------|
| `200` | Successful request |
| `400` | Invalid URL, missing required fields, non-http(s) scheme |
| `401` | Missing or invalid Bearer token |
| `404` | Crawl job ID doesn't exist |
| `422` | LLM extraction failed |
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
| Request timeout | Configurable (default: 60s) |
| URL schemes | `http`, `https` only |
