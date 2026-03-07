# Scraping

crw scrapes single pages via the `POST /v1/scrape` endpoint. It fetches the URL, optionally renders JavaScript, cleans the HTML, and returns content in the requested formats.

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
| `formats` | string[] | `["markdown"]` | Output formats: `markdown`, `html`, `rawHtml`, `plainText`, `links`, `json` |
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

## CSS Selector & XPath Extraction

Extract a specific part of the page before converting to Markdown. Useful for targeting article bodies, tables, or specific DOM elements.

**CSS selector** — uses standard CSS selector syntax:

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

**XPath** — supports XPath 1.0 expressions:

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
| `bm25` | Keyword-heavy queries; fast, no dependencies |
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
