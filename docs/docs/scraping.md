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
  "headers": {}
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

## Content Extraction Pipeline

1. **Fetch** — HTTP request via `reqwest` (or CDP if JS rendering is needed)
2. **Clean** — Remove `script`, `style`, `noscript`, `iframe`, `svg`, `canvas` tags
3. **Main content** — If `onlyMainContent` is true, remove `nav`, `footer`, `header`, `aside`, `menu`
4. **CSS selectors** — Apply `includeTags` / `excludeTags` filters
5. **Readability** — Extract main content block by trying selectors in order: `article` → `main` → `[role="main"]` → `.post-content` → `.article-body` → `.entry-content` → `#content` → `.content` → `body`
6. **Format** — Convert to requested output formats
7. **Metadata** — Extract `title`, `description`, `og:title`, `og:description`, `og:image`, `canonical`, `lang`

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
