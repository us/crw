# Structured Extraction Guide

## Overview

Use extraction when you need shape, not just text. Send `formats: ["json"]` together with a `jsonSchema` to get structured output.

```json
{
  "url": "https://news.ycombinator.com",
  "formats": ["json"],
  "jsonSchema": {
    "type": "object",
    "properties": {
      "stories": {
        "type": "array",
        "items": {
          "type": "object",
          "properties": {
            "title": { "type": "string" },
            "url": { "type": "string" }
          }
        }
      }
    }
  }
}
```

:::note
Firecrawl compatibility: `formats: ["extract"]` is accepted as an alias for `"json"`. Both work identically, but `"json"` is the canonical format name.
:::

## When Extraction Is the Right Tool

Use structured extraction when the downstream consumer expects fields, not prose. Common examples:

- product catalogs,
- article metadata,
- event pages,
- directory listings,
- and pages that will be turned into records for an app or database.

If your next step is retrieval, summarization, or semantic search, markdown is often the better primary output. If your next step is validation, storage, enrichment, or analytics, JSON is usually the better fit.

## End-to-End Request Example

```bash
curl -X POST http://localhost:3000/v1/scrape \
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

Start with the smallest schema that is genuinely useful. Overspecified schemas fail more often and are harder to debug.

## Where It Helps

- e-commerce ingestion,
- article metadata extraction,
- directory parsing,
- and AI workflows that need records rather than prose.

Use extraction when the downstream consumer expects fields, not a markdown blob.

## Designing a Good Schema

Strong extraction schemas share a few traits:

- they ask only for fields that truly matter,
- they avoid ambiguous nested structures when a flat object will do,
- and they match the actual information density of the page.

Good first schema:

```json
{
  "type": "object",
  "properties": {
    "title": { "type": "string" },
    "author": { "type": "string" },
    "publishedAt": { "type": "string" }
  }
}
```

Risky first schema:

```json
{
  "type": "object",
  "properties": {
    "sections": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "title": { "type": "string" },
          "subsections": {
            "type": "array",
            "items": {
              "type": "object",
              "properties": {
                "title": { "type": "string" },
                "bullets": {
                  "type": "array",
                  "items": { "type": "string" }
                }
              }
            }
          }
        }
      }
    }
  }
}
```

The second schema may be valid, but it asks the model to infer a lot of structure that may not exist clearly on the page.

## Bring Your Own Key (BYOK)

You can pass your own LLM API key per-request instead of relying on server configuration:

```json
{
  "url": "https://example.com",
  "formats": ["json"],
  "jsonSchema": {
    "type": "object",
    "properties": {
      "title": { "type": "string" },
      "description": { "type": "string" }
    }
  },
  "llmApiKey": "sk-ant-your-key-here",
  "llmProvider": "anthropic",
  "llmModel": "claude-sonnet-4-20250514"
}
```

| Field | Default | Description |
| --- | --- | --- |
| `llmApiKey` | -- | Your API key. Required if the server has no key configured |
| `llmProvider` | `"anthropic"` | `"anthropic"` or `"openai"` |
| `llmModel` | `"claude-sonnet-4-20250514"` | Model for structured extraction |

When a per-request key is provided, it takes priority over server configuration.

## Operational Advice

Extraction is usually best as a second step:

1. verify the page with markdown first,
2. confirm the target page actually contains the data you want,
3. then layer on the JSON schema.

That saves time when a target page is blocked, incomplete, or structurally noisy.

## Common Mistakes

- **Missing `jsonSchema`**: If you send `formats: ["json"]` without a `jsonSchema`, the API returns a 400 error. You must provide a schema.
- **Wrong format name**: `formats: ["extract"]` works but `"json"` is preferred. `formats: ["llm-extract"]` is also accepted.
- **No LLM configured**: If neither server config nor `llmApiKey` is set, the API returns a 422 error with guidance.
- **Schema too ambitious**: Start with a minimal schema and widen it after you verify extraction quality on real examples.
