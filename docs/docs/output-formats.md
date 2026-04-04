# Output Formats

crw supports 6 output formats. Request multiple formats in a single scrape call.

## Formats

| Format | Key | Description |
|--------|-----|-------------|
| Markdown | `markdown` | HTML converted to Markdown via `fast_html2md` |
| HTML | `html` | Cleaned HTML with main content extraction |
| Raw HTML | `rawHtml` | Original unmodified HTML |
| Plain Text | `plainText` | Stripped to plain text, no formatting |
| Links | `links` | All `<a href>` links extracted (excludes `#` and `javascript:`) |
| JSON | `json` | LLM structured extraction with JSON schema validation |
| Extract | `extract` | Alias for `json` — accepted for Firecrawl compatibility |

## Which Format Should You Choose?

The practical rule is simple:

- choose `markdown` when the output is headed into search, RAG, summarization, or LLM prompts,
- choose `html` when you still want cleaned structure,
- choose `rawHtml` only when you truly need the original source,
- choose `links` when discovery matters as much as page content,
- and choose `json` when the end result needs to be schema-shaped.

For most product and retrieval workflows, `markdown` is the best default because it is compact, readable, and easier to inspect than raw markup.

## Common Format Combinations

| Combination | Good for |
|-------------|----------|
| `["markdown"]` | Default page extraction |
| `["markdown", "links"]` | Content plus local link discovery |
| `["html", "rawHtml"]` | Debugging the extraction pipeline |
| `["json"]` | Structured extraction only |
| `["markdown", "json"]` | Human-readable content plus structured fields |

:::tip
In production, request only the formats you will actually store or process. Requesting more formats is convenient for debugging but adds unnecessary overhead.
:::

## Markdown

The default and most commonly used format. crw uses `fast_html2md` for conversion with a multi-step fallback chain:

1. Convert main-content HTML to Markdown
2. If output is too short, try full cleaned HTML
3. If still short, try without `onlyMainContent`
4. If still short, try raw HTML
5. Last resort: fall back to plain text

This ensures you always get meaningful output, even from unusual page structures.

## HTML

Returns cleaned HTML after the extraction pipeline:
- Scripts, styles, iframes removed
- Navigation, footer, sidebar removed (if `onlyMainContent`)
- CSS selector filters applied (`includeTags` / `excludeTags`)

## Links

Extracts all anchor hrefs from the page. Useful for site mapping and link analysis.

Excluded: `#` fragment-only links and `javascript:` URLs.

## JSON (LLM Extraction)

Sends the page content to an LLM (Anthropic or OpenAI) and extracts structured data matching a JSON schema.

```json
{
  "url": "https://example.com/product",
  "formats": ["json"],
  "jsonSchema": {
    "type": "object",
    "properties": {
      "title": { "type": "string" },
      "price": { "type": "number" }
    }
  }
}
```

The LLM response is validated against the provided schema using the `jsonschema` crate. Markdown fence stripping (`` ```json ``` ``) is handled automatically.

### Supported LLM providers

| Provider | Tool mechanism |
|----------|---------------|
| Anthropic | `tool_use` with `input_schema` |
| OpenAI | Function calling with `parameters` |

Configure in `config.toml`:

```toml
[extraction.llm]
provider = "anthropic"          # or "openai"
api_key = "sk-..."
model = "claude-sonnet-4-20250514"
max_tokens = 4096
# base_url = "https://..."     # for OpenAI-compatible endpoints
```

## Response Shape

Each format populates a corresponding field in the response `data` object:

| Format | Response field | Type |
|--------|---------------|------|
| `markdown` | `markdown` | `string` |
| `html` | `html` | `string` |
| `rawHtml` | `rawHtml` | `string` |
| `plainText` | `plainText` | `string` |
| `links` | `links` | `string[]` |
| `json` / `extract` | `json` | `object` |

## Full Response Schema

Every API response follows this envelope:

```json
{
  "success": true,
  "data": { ... },
  "error": "...",
  "warning": "..."
}
```

The exact shape of `data` depends on what you requested. Do not assume every field is always present.

### `data` object (scrape)

| Field | Type | Present when |
|-------|------|-------------|
| `markdown` | `string / null` | `formats` includes `markdown` or `json` |
| `html` | `string / null` | `formats` includes `html` |
| `rawHtml` | `string / null` | `formats` includes `rawHtml` |
| `plainText` | `string / null` | `formats` includes `plainText` |
| `links` | `string[] / null` | `formats` includes `links` |
| `json` | `object / null` | `formats` includes `json` AND `jsonSchema` provided AND LLM configured |
| `chunks` | `ChunkResult[] / null` | `chunkStrategy` provided |
| `warning` | `string / null` | Target returned error status, anti-bot detected, etc. |
| `metadata` | `object` | Always |

### `metadata` object

| Field | Type | Description |
|-------|------|-------------|
| `title` | `string / null` | Page `<title>` |
| `description` | `string / null` | Meta description |
| `ogTitle` | `string / null` | Open Graph title |
| `ogDescription` | `string / null` | Open Graph description |
| `ogImage` | `string / null` | Open Graph image URL |
| `canonicalUrl` | `string / null` | Canonical link |
| `sourceURL` | `string` | Final URL after redirects |
| `language` | `string / null` | `<html lang>` value |
| `statusCode` | `number` | Target HTTP status code |
| `renderedWith` | `string / null` | `"cdp"`, `"http_only"`, or `"http_only_fallback"` |
| `elapsedMs` | `number` | Total processing time in ms |

### `ChunkResult` object

| Field | Type | Description |
|-------|------|-------------|
| `content` | `string` | Chunk text |
| `score` | `number / null` | Relevance score (present when `query` + `filterMode` set) |
| `index` | `number` | Original chunk position |

## Format Aliases

`"extract"` and `"llm-extract"` are accepted as aliases for `"json"`. The canonical name is `json`. All three behave identically — they require `jsonSchema` for structured extraction.

## Implementation Guidance

Three habits keep format usage sane in production:

- request only the formats you really consume,
- keep `metadata` with the stored output so later debugging is easier,
- and validate `data.json` in your own application before trusting it as final truth.

If you are debugging extraction quality, request both `markdown` and `json` for a while. That makes it easy to compare the page text against the structured output.

## Not Supported in This Release

- `screenshot` — not implemented. Requesting it will return a 422 error.
- `actions` — click/scroll/wait actions are not yet supported. Sending `actions` will return a 400 error with a message suggesting `cssSelector` or `xpath` as alternatives.
