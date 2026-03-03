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
