<div class="page-intro">
  <div class="page-kicker">API Reference</div>
  <h1>Firecrawl v2 Compatibility Reference</h1>
  <p class="page-subtitle">Reference for CRW's <code>/firecrawl/v2/*</code> compatibility routes. New CRW integrations should start with native <code>/v1</code>; use <code>/firecrawl/v2</code> for Firecrawl SDK migrations, batch scraping, PDF parsing, and deprecated extract compatibility.</p>
  <div class="page-capabilities">
    <div class="page-capability"><strong>Base URL:</strong> <code>https://api.fastcrw.com</code></div>
    <div class="page-capability"><strong>Self-hosted:</strong> <code>http://localhost:3000</code></div>
    <div class="page-capability"><strong>Version:</strong> 0.16.0</div>
  </div>
  <div class="page-actions">
    <a class="page-btn primary" href="#post-v2scrape">Start with Scrape</a>
    <a class="page-btn secondary" href="#v2document-response-shape">Response Shape</a>
  </div>
</div>

## Routes at a glance

`/firecrawl/v2` is a compatibility layer, not the recommended API for new CRW builds. Start with `/v1` unless you are migrating existing Firecrawl v2 SDK code.

| Method | Route | Purpose |
|---|---|---|
| `POST` | `/firecrawl/v2/scrape` | Scrape one URL synchronously |
| `GET` | `/firecrawl/v2/scrape/{job_id}` | Stub — always 404 (scrape is synchronous) |
| `POST` | `/firecrawl/v2/crawl` | Start an async recursive crawl |
| `GET` | `/firecrawl/v2/crawl/active` | List in-progress crawl job IDs |
| `GET` | `/firecrawl/v2/crawl/{id}` | Poll crawl status and paginated results |
| `DELETE` | `/firecrawl/v2/crawl/{id}` | Cancel a running crawl |
| `GET` | `/firecrawl/v2/crawl/{id}/errors` | Fetch per-URL errors for a crawl job |
| `POST` | `/firecrawl/v2/map` | Discover URLs, returns link objects |
| `POST` | `/firecrawl/v2/search` | Web search with optional per-result scrape |
| `POST` | `/firecrawl/v2/parse` | Upload a PDF, get markdown or structured JSON |
| `POST` | `/firecrawl/v2/batch/scrape` | Start an async batch scrape over a URL list |
| `GET` | `/firecrawl/v2/batch/scrape/{id}` | Poll batch status and paginated results |
| `DELETE` | `/firecrawl/v2/batch/scrape/{id}` | Cancel a running batch |
| `GET` | `/firecrawl/v2/batch/scrape/{id}/errors` | Fetch per-URL errors for a batch job |
| `POST` | `/firecrawl/v2/extract` | **DEPRECATED** — async multi-URL LLM extraction |
| `GET` | `/firecrawl/v2/extract/{id}` | **DEPRECATED** — poll extract job status |
| `GET` | `/firecrawl/v2/capabilities` | Alias of `/v1/capabilities` |

## v2 vs v1: key differences

| Feature | v1 | v2 |
|---|---|---|
| `formats` field | `string[]` only | `string[]` or `{"type":"...", "schema":...}[]` |
| Map response | `links: string[]` | `links: [{url, title?, description?}[]}` |
| Crawl/batch status | flat | paginated with `next` cursor |
| Document shape | engine-internal | `V2Document` with `metadata.proxyUsed`, `cacheState`, `creditsUsed`, `scrapeId` |
| Crawl status strings | varies | `"scraping"` \| `"completed"` \| `"failed"` |
| `scrapeOptions` in crawl | not present | nested object accepted |
| File parsing | not present | `POST /firecrawl/v2/parse` multipart |
| Batch scrape | not present | `POST /firecrawl/v2/batch/scrape` |

## Authentication

```http
Authorization: Bearer YOUR_API_KEY
```

- Hosted API (`https://api.fastcrw.com`): always required.
- Self-hosted: only required when `auth.api_keys` is configured.
- `/health` is always public.

---

## `POST /firecrawl/v2/scrape`

Scrape one URL synchronously. Returns immediately with a `V2Document`.

### Request body

| Field | Type | Default | Description |
|---|---|---|---|
| `url` | `string` | **required** | Target page URL |
| `formats` | `(string \| FormatObject)[]` | `["markdown"]` | Output formats — see [Formats](#v2-formats) |
| `onlyMainContent` | `boolean` | `true` | Strip nav, footer, sidebar |
| `includeTags` | `string[]` | `[]` | Restrict to these HTML tags |
| `excludeTags` | `string[]` | `[]` | Remove these HTML tags |
| `waitFor` | `number` | — | Milliseconds to wait after JS load |
| `headers` | `Record<string, string>` | `{}` | Custom HTTP headers forwarded to the target |
| `location` | `{ country?: string, languages?: string[] }` | — | Proxy egress country (2-letter ISO) and `Accept-Language` hint |
| `proxy` | `string` | `"auto"` | `"auto"` or `"stealth"` (residential Chrome tier) |
| `proxyList` | `string[]` | `[]` | BYOP proxy URLs (rotated per `proxyRotation`) |
| `proxyRotation` | `"round_robin" \| "random"` | — | Rotation strategy for `proxyList` |
| `timeout` | `number` | server default | Request deadline in milliseconds |
| `renderer` | `"auto" \| "lightpanda" \| "chrome" \| "chrome_proxy" \| "playwright"` | — | Pin a renderer tier |
| `renderJs` | `boolean` | — | `true` forces JS rendering, `false` keeps the request HTTP-only, omitted uses auto-detection. Same semantics as on `/v1/scrape`, see [JS Rendering](js-rendering.md) |
| `parsers` | `ParserSpec[]` | — | Document parser directives (e.g. `["pdf"]`) |
| `llmApiKey` | `string` | — | Per-request LLM API key (required for `summary` / `json` if no server key) |
| `llmProvider` | `"anthropic" \| "openai" \| "deepseek" \| "azure" \| "openai-compatible"` | — | Per-request LLM provider |
| `llmModel` | `string` | — | Per-request LLM model name |
| `summaryPrompt` | `string` | — | Custom prompt for the `summary` format |

### Response

```json
{
  "success": true,
  "data": { "...V2Document..." },
  "warning": "optional — formats not yet supported by this engine"
}
```

See [V2Document Response Shape](#v2document-response-shape) below.

### Example

```bash
curl -X POST https://api.fastcrw.com/firecrawl/v2/scrape \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com",
    "formats": ["markdown", "links"]
  }'
```

Structured extraction with an object format:

```bash
curl -X POST https://api.fastcrw.com/firecrawl/v2/scrape \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://news.ycombinator.com",
    "formats": [
      { "type": "json", "schema": { "type": "object", "properties": { "title": { "type": "string" } } } }
    ]
  }'
```

### TypeScript SDK

```typescript
import { CrwClient } from "crw-sdk";
const crw = new CrwClient({ apiKey: process.env.CRW_API_KEY });

// Simple markdown scrape
const result = await crw.scrape("https://example.com", {
  formats: ["markdown"],
});
console.log(result.markdown);

// Structured extraction
const data = await crw.scrape("https://example.com/product", {
  formats: ["json"],
  jsonSchema: {
    type: "object",
    properties: { title: { type: "string" }, price: { type: "string" } },
  },
});
```

Note: the TypeScript SDK's `scrape()` method calls `/v1/scrape` under the hood; use `batchScrape()` or direct `fetch` to reach the v2 routes explicitly. The `parseFile()` method calls `POST /firecrawl/v2/parse`.

---

## `GET /firecrawl/v2/scrape/{job_id}`

Always returns HTTP 404. CRW scrape is synchronous — there is no deferred job to poll. The endpoint exists only for SDK compatibility; use `POST /firecrawl/v2/scrape` and read the response directly.

---

## `POST /firecrawl/v2/crawl`

Start an asynchronous recursive crawl. Returns a job ID immediately; poll `GET /firecrawl/v2/crawl/{id}` until `status` is `"completed"` or `"failed"`.

### Request body

| Field | Type | Default | Description |
|---|---|---|---|
| `url` | `string` | **required** | Seed URL |
| `limit` | `number` | — | Maximum pages to crawl |
| `maxDiscoveryDepth` | `number` | — | Maximum link-follow depth from the seed |
| `scrapeOptions` | `object` | — | Per-page scrape settings (`formats`, `onlyMainContent`, `waitFor`, `renderJs`) |
| `renderer` | `RequestedRenderer` | — | Pin a renderer tier for all pages |
| `country` | `string` | — | 2-letter ISO country for proxy egress |
| `proxyList` | `string[]` | `[]` | BYOP proxy pool |
| `proxyRotation` | `"round_robin" \| "random"` | — | Rotation strategy for `proxyList` |

### Start response

```json
{
  "success": true,
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "url": "https://api.fastcrw.com/firecrawl/v2/crawl/550e8400-e29b-41d4-a716-446655440000"
}
```

### Example

```bash
# Start the crawl
curl -X POST https://api.fastcrw.com/firecrawl/v2/crawl \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://docs.example.com",
    "limit": 50,
    "maxDiscoveryDepth": 2,
    "scrapeOptions": { "formats": ["markdown"], "onlyMainContent": true }
  }'
```

---

## `GET /firecrawl/v2/crawl/{id}`

Poll crawl status and retrieve completed documents. Supports cursor-based pagination for large crawls.

### Query parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `skip` | `number` | `0` | Zero-based document offset |
| `limit` | `number` | `100` | Maximum documents per page (soft 10 MB byte cap also applies) |

### Status response

```json
{
  "success": true,
  "status": "scraping",
  "total": 50,
  "completed": 12,
  "creditsUsed": 12,
  "expiresAt": "2026-06-16T10:00:00.000Z",
  "next": "https://api.fastcrw.com/firecrawl/v2/crawl/JOB_ID?skip=100",
  "data": [ { "...V2Document..." } ]
}
```

`status` values: `"scraping"` (in progress) | `"completed"` | `"failed"`.

`next` is `null` once the job is `"completed"` and there are no further pages. While the job is still running, `next` is always present so clients keep polling forward.

### Polling pattern

```bash
# Poll until completed
JOB_ID="550e8400-e29b-41d4-a716-446655440000"

while true; do
  RESP=$(curl -s https://api.fastcrw.com/firecrawl/v2/crawl/$JOB_ID \
    -H "Authorization: Bearer YOUR_API_KEY")
  STATUS=$(echo $RESP | jq -r '.status')
  echo "Status: $STATUS, completed: $(echo $RESP | jq '.completed')"
  if [ "$STATUS" = "completed" ] || [ "$STATUS" = "failed" ]; then break; fi
  sleep 3
done
```

---

## `DELETE /firecrawl/v2/crawl/{id}`

Cancel a running crawl. Returns an error if the job has already finished.

```json
{ "success": true, "status": "cancelled", "message": "Crawl job <id> cancelled" }
```

---

## `GET /firecrawl/v2/crawl/active`

List the IDs of all currently in-progress crawl jobs on this engine instance.

```json
{ "success": true, "crawls": ["550e8400-...", "6ba7b810-..."] }
```

---

## `GET /firecrawl/v2/crawl/{id}/errors`

Return per-URL errors accumulated during a crawl.

```json
{
  "success": true,
  "errors": [
    { "id": "550e8400-...", "error": "fetch timeout for https://example.com/slow-page" }
  ],
  "robotsBlocked": []
}
```

---

## `POST /firecrawl/v2/map`

Discover URLs under a domain without scraping their content. The key v2 change from v1: `links` is an array of **objects** (`{url, title?, description?}`), not bare strings.

### Request body

| Field | Type | Default | Description |
|---|---|---|---|
| `url` | `string` | **required** | Base URL to map |
| `limit` | `number` | — | Maximum links to return |
| `includePaths` | `string[]` | `[]` | Substring filters — only keep matching URLs |
| `excludePaths` | `string[]` | `[]` | Substring filters — remove matching URLs |
| `search` | `string` | — | Substring search filter applied after discovery |
| `sitemap` | `"include" \| "only" \| "skip"` | `"include"` | Sitemap strategy |
| `maxDiscoveryDepth` | `number` | server default | Link-follow depth |
| `timeout` | `number` | `120000` | Milliseconds (capped at 300 000) |

### Response

```json
{
  "success": true,
  "links": [
    { "url": "https://example.com/about", "title": null, "description": null },
    { "url": "https://example.com/pricing", "title": null, "description": null }
  ]
}
```

Note: `title` and `description` are always `null` in the current engine version (they are reserved for future sitemap-sourced enrichment).

### Example

```bash
curl -X POST https://api.fastcrw.com/firecrawl/v2/map \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://docs.example.com",
    "limit": 200,
    "includePaths": ["/api", "/reference"]
  }'
```

---

## `POST /firecrawl/v2/search`

Web search returning grouped results (`web`, `news`, `images`). Reuses the same engine as `/v1/search` with a different response envelope.

### Request body

Accepts the same fields as `POST /v1/search`. `scrapeOptions.formats` may be objects (v2 style) or strings (v1 style) — the engine normalizes them automatically.

### Response

```json
{
  "success": true,
  "data": {
    "web": [
      {
        "url": "https://example.com/article",
        "title": "Article Title",
        "description": "Search snippet",
        "position": 1
      }
    ],
    "news": null,
    "images": null
  },
  "creditsUsed": 0,
  "id": "f47ac10b-58cc-4372-a567-0e02b2c3d479"
}
```

`web`, `news`, `images` are omitted when the source did not return results for that category.

### Example

```bash
curl -X POST https://api.fastcrw.com/firecrawl/v2/search \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"query": "fastCRW web scraper", "limit": 5}'
```

---

## `POST /firecrawl/v2/parse`

Upload a PDF file and receive its content as markdown, plain text, or structured JSON. Accepts `multipart/form-data`.

- Maximum upload size: **50 MB**
- Only PDF is supported. Non-PDF uploads receive HTTP 400.

### Multipart fields

| Field | Type | Required | Description |
|---|---|---|---|
| `file` | binary | yes | PDF file bytes |
| `options` | JSON string | no | Serialized `ParseOptions` (see below) |

### ParseOptions (JSON string in the `options` field)

| Field | Type | Default | Description |
|---|---|---|---|
| `formats` | `(string \| FormatObject)[]` | `["markdown"]` | Output formats |
| `jsonSchema` | `object` | — | JSON Schema for structured extraction |
| `parsers` | `ParserSpec[]` | — | Parser directives |
| `summaryPrompt` | `string` | — | Custom prompt for `summary` format |
| `maxContentChars` | `number` | — | Truncate extracted text before LLM steps |

### Response

Same `{ success, data, warning? }` envelope as `POST /firecrawl/v2/scrape`. The `data.metadata.sourceFilename` field carries the original filename; `data.metadata.numPages` carries the page count.

```json
{
  "success": true,
  "data": {
    "markdown": "# Document Title\n\n...",
    "metadata": {
      "sourceURL": "upload://report.pdf",
      "url": "upload://report.pdf",
      "statusCode": 200,
      "proxyUsed": "basic",
      "cacheState": "miss",
      "concurrencyLimited": false,
      "creditsUsed": 1,
      "scrapeId": "a3bb189e-8bf9-3888-9912-ace4e6543002",
      "numPages": 12,
      "sourceFilename": "report.pdf"
    }
  }
}
```

### Example

```bash
curl -X POST https://api.fastcrw.com/firecrawl/v2/parse \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -F "file=@report.pdf" \
  -F 'options={"formats":["markdown"]}'
```

### TypeScript SDK

```typescript
import { readFileSync } from "fs";
import { CrwClient } from "crw-sdk";
const crw = new CrwClient({ apiKey: process.env.CRW_API_KEY });

const pdf = readFileSync("report.pdf");
const result = await crw.parseFile(pdf, { filename: "report.pdf" });
console.log(result.markdown);
```

---

## `POST /firecrawl/v2/batch/scrape`

Start an async job that scrapes a list of URLs with the same scrape options. The job uses the same crawl-job machinery as `/firecrawl/v2/crawl`; the status envelope is identical.

### Request body

| Field | Type | Default | Description |
|---|---|---|---|
| `urls` | `string[]` | **required** | URLs to scrape (at least 1) |
| `formats` | `(string \| FormatObject)[]` | `["markdown"]` | Per-page output formats |
| `ignoreInvalidURLs` | `boolean` | `true` | Skip invalid URLs instead of failing |
| *(any scrape option)* | — | — | Other v2 scrape fields (`onlyMainContent`, `waitFor`, etc.) applied to every page |

### Start response

```json
{
  "success": true,
  "id": "7c9e6679-7425-40de-944b-e07fc1f90ae7",
  "url": "https://api.fastcrw.com/firecrawl/v2/batch/scrape/7c9e6679-...",
  "invalidURLs": []
}
```

### Example

```bash
# Start the batch
curl -X POST https://api.fastcrw.com/firecrawl/v2/batch/scrape \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "urls": [
      "https://example.com/page-1",
      "https://example.com/page-2",
      "https://example.com/page-3"
    ],
    "formats": ["markdown"],
    "onlyMainContent": true
  }'
```

### TypeScript SDK

```typescript
import { CrwClient } from "crw-sdk";
const crw = new CrwClient({ apiKey: process.env.CRW_API_KEY });

const results = await crw.batchScrape(
  ["https://example.com/a", "https://example.com/b"],
  { formats: ["markdown"] }
);
// results is V2Document[]
```

---

## `GET /firecrawl/v2/batch/scrape/{id}`

Poll batch status. Identical envelope to `GET /firecrawl/v2/crawl/{id}` — same `skip`/`limit` query parameters and same `next` cursor pattern.

---

## `DELETE /firecrawl/v2/batch/scrape/{id}`

Cancel a running batch. Delegates to the same handler as `DELETE /firecrawl/v2/crawl/{id}`.

---

## `GET /firecrawl/v2/batch/scrape/{id}/errors`

Fetch per-URL errors. Same response shape as `GET /firecrawl/v2/crawl/{id}/errors`.

---

## Batch start → poll → collect pattern

```
POST /firecrawl/v2/batch/scrape          →  { id }
GET  /firecrawl/v2/batch/scrape/{id}     →  { status, data[], next }
GET  /firecrawl/v2/batch/scrape/{id}?skip=100  →  { status, data[], next }
... (follow `next` until null and status = "completed")
```

The SDK's `batchScrape()` handles this loop automatically. When driving it manually:

1. Start the job, record `id`.
2. Poll `GET /firecrawl/v2/batch/scrape/{id}` every 2–5 seconds.
3. Accumulate `data[]` from each page.
4. If `next` is non-null, follow it; if `next` is null and `status` is `"completed"`, stop.
5. On `status: "failed"`, inspect `/errors`.

---

## `POST /firecrawl/v2/extract` (DEPRECATED)

> **Deprecated.** Use `POST /firecrawl/v2/scrape` with `formats: [{"type": "json", "schema": {...}}]` instead. `/firecrawl/v2/extract` will remain available but the engine emits a `warnings` entry on every response advising the replacement.

Async multi-URL LLM extraction. Starts a job that scrapes each URL with `formats: ["json"]` and the supplied schema, then merges per-URL JSON objects.

### Request body

| Field | Type | Default | Description |
|---|---|---|---|
| `urls` | `string[]` | **required** | URLs to extract from |
| `prompt` | `string` | — | Free-text extraction instruction (no schema required) |
| `schema` | `object` | — | JSON Schema for structured output |
| `systemPrompt` | `string` | — | System-level instruction prepended to the extraction prompt |

### Start response

```json
{
  "success": true,
  "id": "3fa85f64-5717-4562-b3fc-2c963f66afa6",
  "urlTrace": [],
  "warnings": ["/v2/extract is deprecated. Use /v2/scrape with formats including a 'json' format object."],
  "replacement": "/firecrawl/v2/scrape"
}
```

### TypeScript SDK

```typescript
import { CrwClient } from "crw-sdk";
const crw = new CrwClient({ apiKey: process.env.CRW_API_KEY });

// Deprecated path — prefer batchScrape with a json format object
const result = await crw.extract({
  urls: ["https://example.com/product"],
  schema: { type: "object", properties: { title: { type: "string" } } },
});
```

---

## `GET /firecrawl/v2/extract/{id}` (DEPRECATED)

Poll an extract job.

```json
{
  "success": true,
  "status": "completed",
  "data": { "title": "Example Product" },
  "expiresAt": "2026-06-16T10:00:00.000Z",
  "creditsUsed": 1,
  "tokensUsed": 412
}
```

`status` values: `"processing"` (in progress) | `"completed"` | `"failed"`.

---

## V2Document response shape

Every v2 scrape and batch/crawl document follows this shape. Fields are omitted (not null) when not requested.

```json
{
  "markdown": "# Page Title\n\n...",
  "html": "<h1>Page Title</h1>...",
  "rawHtml": "<!doctype html>...",
  "links": ["https://example.com/about"],
  "json": { "title": "Page Title" },
  "summary": "A one-paragraph summary...",
  "changeTracking": { "...ChangeTrackingResult..." },
  "warning": "optional per-document warning",
  "metadata": {
    "title": "Page Title",
    "description": "Page meta description",
    "language": "en",
    "sourceURL": "https://example.com",
    "url": "https://example.com",
    "statusCode": 200,
    "contentType": "text/html; charset=utf-8",
    "proxyUsed": "basic",
    "cacheState": "miss",
    "concurrencyLimited": false,
    "creditsUsed": 1,
    "scrapeId": "a3bb189e-8bf9-3888-9912-ace4e6543002"
  }
}
```

> **Note on `numPages` and `sourceFilename`:** These fields carry `skip_serializing_if = "Option::is_none"` in the engine source, so they are **omitted entirely** from web-scrape responses — they never appear as `null`. They only appear in `/firecrawl/v2/parse` PDF upload responses, and only when the value is actually known.

### V2Document vs v1 ScrapeData

| Field | v1 | v2 |
|---|---|---|
| `metadata.proxyUsed` | not present | `"basic"` or `"stealth"` |
| `metadata.cacheState` | not present | always `"miss"` (no cache yet) |
| `metadata.concurrencyLimited` | not present | always `false` |
| `metadata.creditsUsed` | not present | integer (≥ 1) |
| `metadata.scrapeId` | not present | per-document UUID |
| `metadata.numPages` | not present | page count for PDFs |
| `metadata.sourceFilename` | not present | filename for `/firecrawl/v2/parse` uploads |
| `links` | `string[]` flat | `string[]` inside Document (Map uses objects) |

### `proxyUsed` values

- `"basic"` — default path (lightweight or chrome renderer).
- `"stealth"` — residential Chrome proxy tier, activated by `proxy: "stealth"`.

### `cacheState`

Always `"miss"` in the current engine. The field exists for Firecrawl SDK compatibility.

---

## V2 formats

v2 `formats` accepts a mix of bare strings and typed objects.

### Supported format strings

| Value | Description |
|---|---|
| `"markdown"` | Cleaned markdown (default) |
| `"html"` | Cleaned HTML |
| `"rawHtml"` | Raw full HTML |
| `"plainText"` | Plain text — **note:** the engine computes plain text internally, but `V2Document` has no `plainText` field; requesting this format currently produces no visible output (the value is silently dropped in the v2 serialization layer). |
| `"links"` | Array of discovered URLs |
| `"json"` | Structured JSON (requires schema or LLM key) |
| `"summary"` | LLM-generated summary |
| `"changeTracking"` | Diff against a previous snapshot |

### Object format for JSON extraction

```json
{
  "type": "json",
  "schema": {
    "type": "object",
    "properties": { "title": { "type": "string" } }
  }
}
```

### Object format for changeTracking

```json
{
  "type": "changeTracking",
  "modes": ["gitDiff"],
  "tag": "optional-snapshot-tag"
}
```

### Unsupported formats (graceful warning)

The following formats are recognized but not produced by this engine. The request succeeds with the other requested formats; a `warning` field explains what was skipped:

`images`, `attributes`, `branding`, `audio`, `query`.

`screenshot` IS produced (returned as a `data:image/png;base64,…` URL), but it needs a capture-capable browser tier — Chrome (`chrome`, `chrome_proxy`) or Playwright. Check `screenshot.supported` on `GET /v1/capabilities` first.

---

## V2CrawlStatus (shared by crawl and batch)

```json
{
  "success": true,
  "status": "scraping",
  "total": 50,
  "completed": 12,
  "creditsUsed": 12,
  "expiresAt": "2026-06-16T10:00:00.000Z",
  "next": "https://api.fastcrw.com/firecrawl/v2/crawl/JOB_ID?skip=100",
  "data": [ { "...V2Document..." } ],
  "error": null
}
```

| Field | Description |
|---|---|
| `status` | `"scraping"` \| `"completed"` \| `"failed"` |
| `total` | Estimated total pages (grows as new URLs are discovered) |
| `completed` | Pages fully scraped so far |
| `creditsUsed` | Sum of per-page credit costs |
| `expiresAt` | RFC3339 UTC timestamp when the job record expires |
| `next` | URL for the next result page; `null` when there are no more pages |
| `data` | This page's V2Documents (≤ `limit` documents, soft 10 MB cap) |
| `error` | Set on `"failed"` status |

---

## SDK support

The **TypeScript SDK (`crw-sdk`)** uses v2 by default for the routes it covers:

| SDK method | Underlying route |
|---|---|
| `crw.parseFile()` | `POST /firecrawl/v2/parse` |
| `crw.batchScrape()` | `POST /firecrawl/v2/batch/scrape` + poll |
| `crw.extract()` | `POST /firecrawl/v2/extract` + poll *(deprecated)* |

Other methods (`scrape`, `crawl`, `map`, `search`) call v1 routes. Use direct `fetch` or pass `apiUrl` to target `/firecrawl/v2/scrape` explicitly.

The **Python SDK** exposes the same surface. Pass `base_url="https://api.fastcrw.com"` to the client constructor.

---

## Common mistakes

**Forgetting to poll.** `POST /firecrawl/v2/crawl` and `POST /firecrawl/v2/batch/scrape` return a job ID, not results. The results live in `GET .../{ id }`.

**Expecting `next` to be absent on the last page while the job is running.** `next` is always emitted while `status` is `"scraping"`, even when all buffered pages have been returned. Stop only when `status` is `"completed"` (or `"failed"`) AND `next` is null.

**Sending `formats: ["screenshot"]` without a capture-capable browser.** Screenshots need a Chrome or Playwright tier; LightPanda and Camoufox cannot capture. Check `screenshot.supported` on `GET /v1/capabilities` before you rely on it.

**Using `/firecrawl/v2/extract`.** It works, but the engine warns you to use `POST /firecrawl/v2/scrape` with a `json` format object. Switch over to avoid future breakage.

**Missing `Content-Type: application/json`.** All JSON endpoints require it. `/firecrawl/v2/parse` is multipart — do not set `Content-Type` manually; let the HTTP client set it with the boundary.

---

## What to read next

- [Output Formats](#output-formats) — full format reference
- [Scrape](#scraping) — v1 scrape guide (same engine)
- [Crawl](#crawling) — v1 crawl guide
- [Extract](#extract) — structured extraction patterns
- [Map](#map) — URL discovery
- [Search](#search) — web search
- [Response Shapes](#response-shapes) — v1 shapes
- [Error Codes](#error-codes)
