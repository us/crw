<div class="page-intro">
  <div class="page-kicker">More API</div>
  <h1>Extract</h1>
  <p class="page-subtitle">Return structured JSON from a known page. In CRW, extraction is a scrape mode: use <code>formats: ["json"]</code> and provide a schema that describes the fields you want back.</p>
  <div class="page-capabilities">
    <div class="page-capability"><strong>Best for:</strong> schema-first output</div>
    <div class="page-capability"><strong>Route:</strong> <code>/v1/scrape</code></div>
    <div class="page-capability"><strong>Start with:</strong> a tiny schema</div>
  </div>
  <div class="page-actions">
    <a class="page-btn primary" href="#scraping">View Scrape</a>
    <a class="page-btn secondary" href="https://fastcrw.com/playground" target="_blank" rel="noopener">Try in Playground</a>
  </div>
</div>

<div class="playground-panel">
  <div class="playground-kicker">Try it in the Playground</div>
  <div class="playground-title">Verify the page before you verify the schema</div>
  <div class="playground-copy">A reliable extraction flow is simple: confirm the page scrapes cleanly, then add the smallest possible schema, then expand only if the page really contains the extra fields you want.</div>
  <div class="playground-actions">
    <a class="page-btn primary" href="https://fastcrw.com/playground" target="_blank" rel="noopener">Open Playground</a>
    <a class="page-btn secondary" href="#quick-start">Open Quick Start</a>
  </div>
</div>

## Extracting structured data with CRW

### /v1/scrape

```http
POST /v1/scrape
```

Authentication:

- Hosted: send `Authorization: Bearer YOUR_API_KEY`
- Self-hosted: only required when `auth.api_keys` is configured

### Installation

Extraction uses the same HTTP route as scrape, so you can use the same client code and only change the payload.

### Basic usage

Start with this request:

```json
{
  "url": "https://example.com/product/123",
  "formats": ["json"],
  "jsonSchema": {
    "type": "object",
    "properties": {
      "title": { "type": "string" },
      "price": { "type": "string" },
      "availability": { "type": "string" }
    },
    "required": ["title"]
  }
}
```

:::tabs
::tab{title="Python"}
```python
import requests

resp = requests.post(
    "https://fastcrw.com/api/v1/scrape",
    headers={"Authorization": "Bearer YOUR_API_KEY"},
    json={
        "url": "https://example.com/product/123",
        "formats": ["json"],
        "jsonSchema": {
            "type": "object",
            "properties": {
                "title": {"type": "string"},
                "price": {"type": "string"},
                "availability": {"type": "string"},
            },
            "required": ["title"],
        },
    },
)

print(resp.json()["data"]["json"])
```
::tab{title="Node.js"}
```javascript
const resp = await fetch("https://fastcrw.com/api/v1/scrape", {
  method: "POST",
  headers: {
    "Authorization": "Bearer YOUR_API_KEY",
    "Content-Type": "application/json"
  },
  body: JSON.stringify({
    url: "https://example.com/product/123",
    formats: ["json"],
    jsonSchema: {
      type: "object",
      properties: {
        title: { type: "string" },
        price: { type: "string" },
        availability: { type: "string" }
      },
      required: ["title"]
    }
  })
});

const body = await resp.json();
console.log(body.data.json);
```
::tab{title="cURL"}
```bash
curl -X POST https://fastcrw.com/api/v1/scrape \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "url":"https://example.com/product/123",
    "formats":["json"],
    "jsonSchema":{
      "type":"object",
      "properties":{
        "title":{"type":"string"},
        "price":{"type":"string"},
        "availability":{"type":"string"}
      },
      "required":["title"]
    }
  }'
```
:::

### Response

```json
{
  "success": true,
  "data": {
    "json": {
      "title": "Widget",
      "price": "$19.99",
      "availability": "In stock"
    },
    "metadata": {
      "sourceURL": "https://example.com/product/123",
      "statusCode": 200,
      "elapsedMs": 846
    }
  }
}
```

## Parameters

| Field | Type | Default | Description |
|---|---|---|---|
| `url` | string | required | Target page URL |
| `formats` | string[] | required | Use `["json"]` for the canonical extraction path |
| `jsonSchema` | object | required | JSON schema describing the fields you want back |
| `extract` | object | -- | Firecrawl-compatible wrapper; `extract.schema` is accepted |
| `llmApiKey` | string | -- | Per-request BYOK credential |
| `llmProvider` | string | server default | `anthropic` or `openai` |
| `llmModel` | string | server default | Extraction model override |
| `onlyMainContent` | boolean | `true` | Keep extraction focused on the main content block |
| `cssSelector` | string | -- | Narrow the page before extraction |
| `xpath` | string | -- | Narrow the page before extraction |

:::note
Firecrawl-compatible aliases `formats: ["extract"]` and `formats: ["llm-extract"]` are accepted, but `["json"]` is the canonical CRW shape.
:::

:::warning
On self-hosted deployments, JSON extraction requires either `[extraction.llm]` in `config.toml` or a per-request `llmApiKey`. Without one of those, CRW returns an extraction error instead of guessed output.
:::

## Schema design

The easiest extraction schema is a small one:

- ask for the fields you actually need,
- keep field types simple on the first pass,
- avoid optional fields until the required ones are stable.

Large schemas are harder to debug because you cannot tell whether the issue is the page, the schema, or both.

## Extraction quality

Use this workflow:

1. scrape the page as markdown,
2. verify the target content is present,
3. add a minimal schema,
4. expand the schema only after the first JSON result looks right.

If the underlying page scrape is weak, the JSON extraction will also be weak.

## BYOK and provider control

Use `llmApiKey`, `llmProvider`, and `llmModel` only when you need per-request control. For most users, server defaults are the right starting point once `[extraction.llm]` is configured.

## Common production patterns

- Validate the target with markdown first, then add extraction.
- Keep the schema as small as possible on the first pass.
- Narrow the page with `cssSelector` only when the default extraction is too noisy.
- Use BYOK only when you need per-request provider separation.

## Common mistakes

- Sending `formats: ["json"]` without a schema
- Designing a schema that assumes more structure than the page really contains
- Debugging extraction before confirming the underlying scrape succeeded
- Treating extraction as a separate route instead of a scrape mode

## When to use something else

- Use [Scrape](#scraping) when prose or markdown is enough
- Use [Map](#map) when you need discovery before extraction
- Use [Crawl](#crawling) when you need schema-based extraction across many pages
