# Web Scraping API

CRW scrapes single pages via the `POST /v1/scrape` endpoint — a Firecrawl-compatible web scraping API. It fetches the URL, optionally renders JavaScript via LightPanda or Chrome, cleans the HTML, and returns content in your requested formats (Markdown, HTML, JSON, plain text).

Use `scrape` when you want one page turned into usable content without starting a wider crawl job. It is the right default for:

- first-pass evaluation,
- RAG ingestion from known URLs,
- extraction pipelines,
- and agent workflows that already know which page to fetch.

## Request

```
POST /v1/scrape
```

```json
{
  "url": "https://example.com",
  "formats": ["markdown"],
  "onlyMainContent": true,
  "renderJs": null,
  "waitFor": 2000,
  "includeTags": [],
  "excludeTags": [],
  "jsonSchema": null,
  "headers": {},
  "cssSelector": null,
  "xpath": null,
  "chunkStrategy": null,
  "query": null,
  "filterMode": null,
  "topK": 5,
  "proxy": null,
  "stealth": null
}
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `url` | string | required | URL to scrape |
| `formats` | string[] | `["markdown"]` | Output formats: `markdown`, `html`, `rawHtml`, `plainText`, `links`, `json`, `extract` |
| `onlyMainContent` | bool | `true` | Extract only main content (removes nav, footer, sidebar, etc.) |
| `renderJs` | bool/null | `null` | `null` = auto-detect, `true` = force CDP, `false` = HTTP only |
| `waitFor` | int | — | Wait time in ms after JS rendering |
| `includeTags` | string[] | `[]` | CSS selectors to keep (whitelist) |
| `excludeTags` | string[] | `[]` | CSS selectors to remove (blacklist) |
| `jsonSchema` | object | — | JSON schema for LLM structured extraction |
| `headers` | object | `{}` | Custom HTTP headers to send |
| `cssSelector` | string | — | Extract only elements matching this CSS selector |
| `xpath` | string | — | Extract only elements matching this XPath expression |
| `chunkStrategy` | object | — | Chunk strategy: `{"type":"topic"}`, `{"type":"sentence","maxChars":500}`, `{"type":"regex","pattern":"\\n\\n"}` |
| `query` | string | — | Query for BM25/cosine chunk ranking |
| `filterMode` | string | — | `"bm25"` or `"cosine"` |
| `topK` | int | `5` | Number of top chunks to return |
| `proxy` | string | — | Per-request proxy (e.g. `"http://user:pass@host:8080"`) |
| `stealth` | bool | — | Override global stealth setting for this request |
| `llmApiKey` | string | — | Per-request LLM API key for structured extraction (BYOK). Overrides server config. Available on [fastcrw.com](https://fastcrw.com) (cloud) |
| `llmProvider` | string | `"anthropic"` | LLM provider: `"anthropic"` or `"openai"` |
| `llmModel` | string | `"claude-sonnet-4-20250514"` | Model to use for structured extraction |

## A Good Default Request

If you are not sure where to start, use this shape first:

```json
{
  "url": "https://example.com",
  "formats": ["markdown"],
  "onlyMainContent": true,
  "renderJs": null
}
```

That gives you a clean markdown output, keeps extraction focused on the main body, and leaves JavaScript rendering to the engine's default behavior.

## Choosing the Right Formats

Most integrations only need one of these patterns:

- `["markdown"]` for retrieval, search, summarization, and LLM inputs.
- `["markdown", "links"]` when you want the content plus outbound link discovery.
- `["html"]` when you need cleaned markup instead of markdown.
- `["rawHtml"]` when downstream logic expects the original HTML source.
- `["json"]` when you are doing schema-driven extraction.

Requesting more formats is convenient for debugging, but in production it is better to ask only for what you will actually store or process.

## Content Extraction Pipeline

1. **Fetch** — HTTP request via `reqwest` (or CDP if JS rendering is needed)
2. **Clean** — Remove `script`, `style`, `noscript`, `iframe`, `svg`, `canvas` tags
3. **Selector** — If `cssSelector` or `xpath` is set, narrow the DOM to matching elements (readability is skipped)
4. **Main content** — If `onlyMainContent` is true and no selector was given, remove `nav`, `footer`, `header`, `aside`, `menu`
5. **CSS filters** — Apply `includeTags` / `excludeTags` allow/deny lists
6. **Readability** — Extract main content block: `article` → `main` → `[role="main"]` → `.post-content` → `.article-body` → `.entry-content` → `#content` → `.content` → `body`
7. **Format** — Convert to requested output formats (htmd for Markdown)
8. **Chunking** — If `chunkStrategy` is set, split Markdown into chunks; rank by `query` if `filterMode` is provided
9. **Metadata** — Extract `title`, `description`, `og:title`, `og:description`, `og:image`, `canonical`, `lang`

## Auto JS Detection

When `renderJs` is `null` (default), crw uses heuristics to decide whether JS rendering is needed:

- Body text is under 200 characters **and** contains SPA markers (`id="root"`, `id="__next"`, `data-reactroot`, etc.)
- `<noscript>` tag contains "enable javascript"
- Body text is under 500 characters and URL matches known SPA hosts (Framer, Webflow, Wix, Squarespace)

If any heuristic matches, crw re-fetches the page using CDP.

## JS Rendering Guidance

Use `renderJs: true` only when the page clearly needs a browser. Browser rendering increases latency and operational cost, so treat it as a deliberate choice rather than the universal default.

When you do need it:

- set `renderJs: true`,
- start with `waitFor: 1000` or `2000`,
- and raise `waitFor` only when the page still hydrates too slowly.

If the response metadata shows an HTTP-only fallback or the output is suspiciously empty, read the [JS rendering guide](/docs/js-rendering).

## LLM Structured Extraction

Use the `json` format with a `jsonSchema` to extract structured data using an LLM:

```json
{
  "url": "https://example.com/products/widget",
  "formats": ["json"],
  "jsonSchema": {
    "type": "object",
    "properties": {
      "name": { "type": "string" },
      "price": { "type": "number" },
      "features": {
        "type": "array",
        "items": { "type": "string" }
      }
    },
    "required": ["name", "price"]
  }
}
```

crw sends the page content to the configured LLM provider (Anthropic or OpenAI) and validates the response against the JSON schema.

Configure the LLM provider in `config.toml`:

```toml
[extraction.llm]
provider = "anthropic"
api_key = "sk-ant-..."
model = "claude-sonnet-4-20250514"
max_tokens = 4096
```

You do not need a separate endpoint for extraction. `scrape` can also return schema-shaped JSON when `formats` includes `json` and `jsonSchema` is present. That means a single API surface can support markdown for retrieval, links for discovery, and JSON for downstream application logic.

## Targeting the Right Part of a Page

The default extraction path works well for many pages, but if you know the site structure, tighten the request:

- use `cssSelector` when there is a stable content container,
- use `xpath` when selectors are easier to express that way,
- use `includeTags` and `excludeTags` to keep or remove specific markup families,
- and leave `onlyMainContent` on unless you explicitly want navigation, footer, or sidebar content.

:::tip
The common mistake is combining too many narrowing options at once. Start broad, inspect the result, then add one targeting primitive at a time.
:::

### CSS Selector

Uses standard CSS selector syntax:

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://news.ycombinator.com",
    "formats": ["markdown"],
    "cssSelector": "td.title",
    "onlyMainContent": false
  }'
```

### XPath

Supports XPath 1.0 expressions:

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://news.ycombinator.com",
    "formats": ["markdown"],
    "xpath": "//span[@class='\''titleline'\'']/a",
    "onlyMainContent": false
  }'
```

When a selector is active, the readability pass and `onlyMainContent` filtering are bypassed — only the selected HTML is converted. Multiple matched elements are concatenated.

## Chunking

Split scraped Markdown into chunks for vector databases and RAG pipelines. Results are returned in `data.chunks`.

### Strategies

**Topic** — split on Markdown headings, keeping each section as one chunk:

```json
{ "chunkStrategy": { "type": "topic" } }
```

**Sentence** — split on sentence boundaries (`.`, `!`, `?`), merge until `maxChars` is reached:

```json
{ "chunkStrategy": { "type": "sentence", "maxChars": 500 } }
```

**Regex** — split on a custom delimiter pattern:

```json
{ "chunkStrategy": { "type": "regex", "pattern": "\\n\\n" } }
```

### Chunking & Filtering Behavior

- `chunkStrategy` alone splits the markdown and returns all chunks.
- `chunkStrategy` + `query` + `filterMode` scores and ranks chunks, returning the top `topK`.
- `topK` without `query`/`filterMode` still truncates the chunk array to `topK` items (no scoring).
- `query` or `filterMode` without `chunkStrategy` is silently ignored — chunking must be enabled first.

In practice:

- use `sentence` when you want stable natural-language chunks,
- use `regex` when you already know the structural separator,
- and treat `topic` chunking as an advanced option that should be tested on real data before wide rollout.

### Chunk Filtering (BM25 / Cosine)

Rank chunks by relevance to a query and return the top K:

```json
{
  "url": "https://en.wikipedia.org/wiki/Rust_(programming_language)",
  "formats": ["markdown"],
  "onlyMainContent": true,
  "chunkStrategy": { "type": "topic" },
  "query": "memory safety ownership borrow checker",
  "filterMode": "bm25",
  "topK": 5
}
```

| `filterMode` | When to use |
|---|---|
| `bm25` | Keyword-heavy queries; fast, no dependencies. Recommended for most use cases |
| `cosine` | Semantic overlap; uses TF-IDF vectors |

Without `filterMode`, all chunks are returned in document order.

## Stealth Mode

Inject browser-like HTTP headers to reduce bot-detection fingerprinting. Enable per request or globally.

**Per request:**

```json
{
  "url": "https://example.com",
  "stealth": true
}
```

**Global default** in `config.toml`:

```toml
[crawler]
stealth = true
```

When stealth is active, CRW:
- Rotates the User-Agent from a pool of real Chrome 131, Firefox 133, and Safari 18 strings
- Injects `Accept`, `Accept-Language`, `Accept-Encoding`, `Sec-Ch-Ua`, `Sec-Ch-Ua-Mobile`, `Sec-Ch-Ua-Platform`, `Sec-Fetch-Dest`, `Sec-Fetch-Mode`, `Sec-Fetch-Site`, `Sec-Fetch-User`, `Priority`, and `Upgrade-Insecure-Requests` headers

## Per-Request Proxy

Override the global proxy for a single request:

```json
{
  "url": "https://example.com",
  "proxy": "http://user:pass@proxy-host:8080"
}
```

The global proxy is configured in `config.toml`:

```toml
[crawler]
proxy = "http://proxy-host:8080"
```

## Response Semantics

The main response pattern is:

- `success` for overall request outcome,
- `data` for returned content,
- `warning` for degraded but non-fatal situations,
- and `metadata` for context such as title, status code, final URL, and elapsed time.

:::warning
Do not ignore warnings. A page blocked by anti-bot protection can still produce content that looks valid at first glance.
:::
