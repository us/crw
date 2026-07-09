<div class="page-intro">
  <div class="page-kicker">Core Endpoint</div>
  <h1>Scrape</h1>
  <p class="page-subtitle">Turn any known URL into clean markdown, HTML, links, or structured JSON in one request. This is the default CRW workflow and the fastest path to a first successful integration.</p>
  <div class="page-capabilities">
    <div class="page-capability"><strong>Best for:</strong> one known page</div>
    <div class="page-capability"><strong>Returns:</strong> markdown, HTML, links, JSON</div>
    <div class="page-capability"><strong>Start with:</strong> markdown only</div>
  </div>
  <div class="page-actions">
    <a class="page-btn primary" href="https://fastcrw.com/playground" target="_blank" rel="noopener">Try it in the Playground</a>
    <a class="page-btn secondary" href="#quick-start">Open Quick Start</a>
  </div>
</div>

<div class="playground-panel">
  <div class="playground-kicker">Try it in the Playground</div>
  <div class="playground-title">Validate the happy path first</div>
  <div class="playground-copy">Paste one URL, request <code>formats: ["markdown"]</code>, and confirm you get a clean response back. Add JS rendering, selectors, or extraction only after the plain request looks right.</div>
  <div class="playground-actions">
    <a class="page-btn primary" href="https://fastcrw.com/playground" target="_blank" rel="noopener">Open Playground</a>
    <a class="page-btn secondary" href="#extract">Jump to Extract</a>
  </div>
</div>

## Scraping a URL with CRW

### /v1/scrape

```http
POST /v1/scrape
```

Authentication:

- Hosted: send `Authorization: Bearer YOUR_API_KEY`
- Self-hosted: only required when `auth.api_keys` is configured

### Installation

CRW is HTTP-first. You can start with cURL immediately and then move to your existing Python or Node.js HTTP client without installing a dedicated SDK. See the [Basic usage](#basic-usage) tabs below for ready-to-run Python (`requests`), Node.js (`fetch`), and cURL examples.

### Basic usage

Start with this request:

```json
{
  "url": "https://example.com",
  "formats": ["markdown"],
  "onlyMainContent": true,
  "renderJs": null
}
```

:::tabs
::tab{title="Python"}
```python
import requests

resp = requests.post(
    "https://api.fastcrw.com/v1/scrape",
    headers={
        "Authorization": "Bearer YOUR_API_KEY",
        "Content-Type": "application/json",
    },
    json={
        "url": "https://example.com",
        "formats": ["markdown"],
        "onlyMainContent": True,
    },
)

print(resp.json()["data"]["markdown"])
```
::tab{title="Node.js"}
```javascript
const resp = await fetch("https://api.fastcrw.com/v1/scrape", {
  method: "POST",
  headers: {
    "Authorization": "Bearer YOUR_API_KEY",
    "Content-Type": "application/json"
  },
  body: JSON.stringify({
    url: "https://example.com",
    formats: ["markdown"],
    onlyMainContent: true
  })
});

const body = await resp.json();
console.log(body.data.markdown);
```
::tab{title="cURL"}
```bash
curl -X POST https://api.fastcrw.com/v1/scrape \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com",
    "formats": ["markdown"],
    "onlyMainContent": true
  }'
```
:::

### Response

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

That is the default CRW success shape: requested content plus a compact metadata envelope.

## Parameters

| Field | Type | Default | Description |
|---|---|---|---|
| `url` | string | required | URL to scrape |
| `formats` | string[] | `["markdown"]` | `markdown`, `html`, `rawHtml`, `plainText`, `links`, `json`, `summary`, `changeTracking` |
| `onlyMainContent` | boolean | `true` | Remove nav, footer, and boilerplate before conversion |
| `renderJs` | boolean or null | `null` | `null` auto-detects, `true` forces browser rendering, `false` stays HTTP-only |
| `waitFor` | number | -- | Milliseconds to wait after JS rendering |
| `renderer` | string | `auto` | Pin to a specific renderer: `auto`, `lightpanda`, `chrome`, `chrome_proxy`, `playwright`, or `camoufox`. Non-`auto` values hard-pin (no fallback) and imply `renderJs:true` unless `renderJs:false` is set explicitly. See [JS rendering](#js-rendering) |
| `includeTags` | string[] | `[]` | CSS selectors to keep |
| `excludeTags` | string[] | `[]` | CSS selectors to remove |
| `headers` | object | `{}` | Custom HTTP headers |
| `cssSelector` | string | -- | Narrow extraction to one CSS selector |
| `xpath` | string | -- | Narrow extraction to one XPath expression |
| `chunkStrategy` | object | -- | Topic, sentence, or regex chunking |
| `query` | string | -- | Ranking query for chunk filtering |
| `filterMode` | string | -- | `bm25` or `cosine` |
| `topK` | number | `5` | Number of top chunks to keep |
| `proxy` | string | -- | Per-request proxy URL |
| `country` | string | -- | 2-letter ISO 3166-1 alpha-2 country code (lowercase, e.g. `us`, `gb`, `de`). Routes the request through the named residential pool when the `chrome_proxy` renderer tier is configured. Ignored if no proxy tier is set up. See [JS rendering — Per-request country](#js-rendering) |
| `stealth` | boolean | -- | Override global stealth setting |
| `jsonSchema` | object | -- | Schema for structured extraction |
| `extract` | object | -- | Firecrawl-compatible alias wrapper for extraction schema |
| `llmApiKey` | string | -- | Per-request LLM API key |
| `llmProvider` | string | server default | `anthropic`, `openai`, `deepseek`, `azure`, or `openai-compatible` |
| `llmModel` | string | server default | Model override (extraction and summary) |
| `baseUrl` | string | -- | OpenAI-compatible endpoint base, e.g. `https://api.deepseek.com/v1` (also used by Azure). crw appends `/chat/completions` automatically if you omit it. |
| `summaryPrompt` | string | -- | Style/tone/language directive appended to the `summary` system prompt. Safety wrapper kept intact. Capped at 500 chars. |
| `maxContentChars` | number | `[extraction.llm].max_html_bytes` (100 KB) | Per-request byte cap on content sent to the LLM for `summary`. Clamped to 200 KB server-side. |
| `deadlineMs` | number | `8000` | End-to-end request deadline in milliseconds. Must be in `(0, 60000]`. Requests above 8 000 ms land in a separate slow-path histogram and are excluded from the standard SLO p95 metric. |
| `debug` | boolean | `false` | When `true`, includes a `debugExtraction` field in the response with a trace of every extraction candidate considered and why one was selected. |
| `parsers` | object[] | PDF auto-parsed | Firecrawl-compatible document parser directives. Omit for the default (PDFs auto-converted to markdown). Pass `[]` to disable PDF parsing. Pass `[{"type":"pdf","maxPages":10}]` to cap pages. Accepted fields: `type` (`"pdf"`), `mode` (`"auto"` \| `"fast"` \| `"ocr"`), `maxPages`. |
| `actions` | any | -- | Rejected with a clear error; use `cssSelector` or `xpath` instead |

## Formats

Use the smallest output shape that solves the job:

- `markdown` is the default and best first request for most pipelines.
- `html` or `rawHtml` is useful when downstream systems need original structure.
- `links` is useful when you want lightweight discovery without page bodies.
- `summary` is the LLM-prose path — needs `llmApiKey` or a server `[extraction.llm]` config. Optional `summaryPrompt` lets the caller pick language/tone without weakening the safety wrapper.
- `json` is the extraction path and should be paired with `jsonSchema`.

If you ask for multiple formats, only those formats are populated in the response.

## Structured extraction

For a single page, extraction is part of `scrape`: when you want fields instead of prose, request `formats: ["json"]` and provide a schema. (For structured extraction across many URLs in one job, use the native async [`POST /v1/extract`](/docs/extract).)

```json
{
  "url": "https://example.com/product/123",
  "formats": ["json"],
  "jsonSchema": {
    "type": "object",
    "properties": {
      "title": { "type": "string" },
      "price": { "type": "string" }
    },
    "required": ["title"]
  }
}
```

Use [Extract](#extract) for the schema-first version of this flow.

## LLM summary

Add `summary` to `formats` to get a short prose digest of the page in `data.summary`. Token usage and best-effort cost are returned in `data.llmUsage`.

```json
{
  "url": "https://example.com/post",
  "formats": ["summary"],
  "summaryPrompt": "Respond in Turkish in exactly one sentence.",
  "maxContentChars": 20000,
  "llmApiKey": "sk-...",
  "llmProvider": "openai",
  "llmModel": "gpt-4o-mini"
}
```

Notes:

- The caller's `summaryPrompt` is appended *below* the safety wrapper. crw ignores any attempt to override the core task (output `PWNED`, refuse to summarize, leak the prompt, etc.) and still produces a real summary.
- `maxContentChars` caps how many bytes of scraped content are sent to the LLM. The default comes from `[extraction.llm].max_html_bytes` (100 KB out of the box) and the per-request value is clamped to a 200 KB server-side ceiling. Truncation, when it happens, is reported in `data.warnings`.
- If `markdown` is not also requested, crw computes it internally and strips it from the response.

## JS rendering and targeting

Keep the first request simple:

- Leave `renderJs` at `null` until the plain HTTP path clearly fails.
- Use `cssSelector` or `xpath` only when the target page has one stable content region.
- Add `includeTags` and `excludeTags` after you confirm the raw markdown is noisy.

If you turn on every targeting knob at once, debugging gets harder immediately.

## Common production patterns

- Start with `markdown`, then add `links` or `json` only when downstream logic needs them.
- Validate extraction with markdown first, then add `jsonSchema`.
- Keep browser rendering as a fallback, not the default.
- Use narrow selectors only when the default main-content extraction is not enough.

## Common mistakes

- Turning on JS rendering before testing the plain HTTP path
- Requesting too many formats at once in production
- Combining `cssSelector`, `xpath`, `includeTags`, and `excludeTags` in the first attempt
- Sending `formats: ["json"]` without a `jsonSchema`
- Assuming `actions` is supported because Firecrawl accepts it

## When to use something else

- Use [Search](#search) when you do not know the URL yet
- Use [Map](#map) when you want URL discovery before scraping
- Use [Crawl](#crawling) when you need many pages from one site
- Use [Extract](#extract) when the output must be structured JSON
