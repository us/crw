---
title: API Reference
layout: default
nav_order: 4
description: "CRW API reference: scrape, crawl, map endpoints with request/response schemas and curl examples."
---

# API Reference
{: .no_toc }

All endpoints, request/response schemas, and examples.
{: .fs-6 .fw-300 }

## Table of Contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Base URL

```
http://localhost:3000
```

All `/v1/*` endpoints require authentication when API keys are configured. The `/health` endpoint is always public.

---

## Health Check

```
GET /health
```

Returns server status and renderer availability. No authentication required.

### Example

```bash
curl http://localhost:3000/health
```

### Response

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

| Field | Type | Description |
|:------|:-----|:------------|
| `status` | string | Always `"ok"` if server is running |
| `version` | string | Server version |
| `renderers` | object | Map of renderer name → availability |
| `active_crawl_jobs` | number | Currently running crawl jobs |

---

## Scrape

```
POST /v1/scrape
```

Scrape a single URL and extract content in one or more formats.

### Request Body

| Field | Type | Required | Default | Description |
|:------|:-----|:---------|:--------|:------------|
| `url` | string | **yes** | — | URL to scrape (`http`/`https` only) |
| `formats` | string[] | no | `["markdown"]` | Output formats (see below) |
| `onlyMainContent` | boolean | no | `true` | Strip navigation, footer, sidebar |
| `renderJs` | boolean\|null | no | `null` | `null`=auto, `true`=force JS, `false`=HTTP only |
| `waitFor` | number | no | — | Milliseconds to wait after JS rendering |
| `includeTags` | string[] | no | `[]` | CSS selectors — only include matching elements |
| `excludeTags` | string[] | no | `[]` | CSS selectors — remove matching elements |
| `headers` | object | no | `{}` | Custom HTTP request headers |
| `jsonSchema` | object | no | — | JSON Schema for LLM structured extraction |

**Output formats:**

| Format | Description |
|:-------|:------------|
| `markdown` | Cleaned HTML converted to Markdown |
| `html` | Cleaned HTML (scripts, styles, ads removed) |
| `rawHtml` | Original HTML as-is |
| `plainText` | Text content only, no markup |
| `links` | Array of all links found on the page |
| `json` | LLM-extracted structured data (requires `jsonSchema` + LLM config) |

{: .note }
Snake case aliases are also accepted: `only_main_content`, `render_js`, `wait_for`, `include_tags`, `exclude_tags`, `json_schema`.

### Response Body

```json
{
  "success": true,
  "data": {
    "markdown": "string or null",
    "html": "string or null",
    "rawHtml": "string or null",
    "plainText": "string or null",
    "links": ["string"] or null,
    "json": {} or null,
    "metadata": {
      "title": "string or null",
      "description": "string or null",
      "ogTitle": "string or null",
      "ogDescription": "string or null",
      "ogImage": "string or null",
      "canonicalUrl": "string or null",
      "sourceURL": "string (always present)",
      "language": "string or null",
      "statusCode": 200,
      "renderedWith": "string or null",
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

```json
{
  "success": true,
  "data": {
    "markdown": "# Example Domain\nThis domain is for use in documentation examples without needing permission. Avoid use in operations.\n[Learn more](https://iana.org/domains/example)",
    "metadata": {
      "title": "Example Domain",
      "sourceURL": "https://example.com",
      "language": "en",
      "statusCode": 200,
      "elapsedMs": 32
    }
  }
}
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

```json
{
  "success": true,
  "data": {
    "markdown": "# Example Domain\nThis domain is for use in documentation examples without needing permission. Avoid use in operations.\n[Learn more](https://iana.org/domains/example)",
    "html": "<div><h1>Example Domain</h1><p>This domain is for use in documentation examples without needing permission. Avoid use in operations.</p><p><a href=\"https://iana.org/domains/example\">Learn more</a></p></div>\n",
    "links": [
      "https://iana.org/domains/example"
    ],
    "metadata": {
      "title": "Example Domain",
      "sourceURL": "https://example.com",
      "language": "en",
      "statusCode": 200,
      "elapsedMs": 20
    }
  }
}
```

**With CSS selectors:**

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com",
    "excludeTags": ["nav", "footer", ".sidebar"]
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

---

## Crawl

### Start a Crawl

```
POST /v1/crawl
```

Start an async BFS crawl. Returns immediately with a job ID.

#### Request Body

| Field | Type | Required | Default | Description |
|:------|:-----|:---------|:--------|:------------|
| `url` | string | **yes** | — | Starting URL (`http`/`https` only) |
| `maxDepth` | number | no | `2` | Maximum link-follow depth |
| `maxPages` | number | no | `100` | Maximum pages to scrape |
| `formats` | string[] | no | `["markdown"]` | Output formats for each page |
| `onlyMainContent` | boolean | no | `true` | Strip boilerplate from each page |

#### Example

```bash
curl -X POST http://localhost:3000/v1/crawl \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com",
    "maxDepth": 1,
    "maxPages": 2
  }'
```

#### Response

```json
{
  "success": true,
  "id": "a4c03342-ab36-4df6-9e15-7ecffc9f8b3a"
}
```

### Check Crawl Status

```
GET /v1/crawl/:id
```

Poll a crawl job for status and results.

#### Example

```bash
curl http://localhost:3000/v1/crawl/a4c03342-ab36-4df6-9e15-7ecffc9f8b3a
```

#### Response

```json
{
  "status": "completed",
  "total": 1,
  "completed": 1,
  "data": [
    {
      "markdown": "# Example Domain\nThis domain is for use in documentation examples without needing permission. Avoid use in operations.\n[Learn more](https://iana.org/domains/example)",
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

#### Status Values

| Status | Description |
|:-------|:------------|
| `scraping` | Crawl is in progress |
| `completed` | All pages scraped successfully |
| `failed` | Crawl encountered a fatal error |

#### Response Fields

| Field | Type | Description |
|:------|:-----|:------------|
| `status` | string | Current crawl status |
| `total` | number | Total URLs discovered |
| `completed` | number | Pages scraped so far |
| `data` | array | Array of scrape results (same format as `/v1/scrape` data) |
| `error` | string\|null | Error message if failed |

{: .note }
Completed crawl jobs are automatically cleaned up after the configured TTL (default: 1 hour).

---

## Map

```
POST /v1/map
```

Discover all URLs on a website via crawling and sitemap parsing.

### Request Body

| Field | Type | Required | Default | Description |
|:------|:-----|:---------|:--------|:------------|
| `url` | string | **yes** | — | URL to discover links from |
| `maxDepth` | number | no | `2` | Maximum discovery depth |
| `useSitemap` | boolean | no | `true` | Also read sitemap.xml |

### Example

```bash
curl -X POST http://localhost:3000/v1/map \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'
```

### Response

```json
{
  "success": true,
  "links": [
    "https://example.com"
  ]
}
```

---

## Error Responses

All errors return the same format:

```json
{
  "success": false,
  "error": "Human-readable error message"
}
```

### HTTP Status Codes

| Status | Meaning | When |
|:-------|:--------|:-----|
| `200` | OK | Successful request |
| `400` | Bad Request | Invalid URL, missing required fields, non-http(s) scheme |
| `401` | Unauthorized | Missing or invalid Bearer token |
| `404` | Not Found | Crawl job ID doesn't exist |
| `422` | Unprocessable Entity | LLM extraction failed |
| `502` | Bad Gateway | Target website returned an error |
| `504` | Gateway Timeout | Request timed out |
| `500` | Internal Server Error | Unexpected server error |
