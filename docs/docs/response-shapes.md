# Response Shapes

Use this page when you want the common CRW envelopes in one place. The endpoint pages stay the source of truth for behavior; this page is the quick shape reference.

## Scrape

```json
{
  "success": true,
  "data": {
    "markdown": "string or null",
    "html": "string or null",
    "rawHtml": "string or null",
    "plainText": "string or null",
    "links": ["string"],
    "json": {},
    "summary": "string or null",
    "llmUsage": {
      "inputTokens": 1234,
      "outputTokens": 567,
      "totalTokens": 1801,
      "estimatedCostUsd": 0.00023,
      "model": "gpt-4o-mini",
      "provider": "openai"
    },
    "chunks": [
      {
        "content": "string",
        "score": 0.91,
        "index": 0
      }
    ],
    "warnings": ["content truncated to 100000 bytes before summarization"],
    "warning": "optional warning",
    "metadata": {
      "title": "string",
      "description": "string",
      "sourceURL": "https://example.com",
      "statusCode": 200,
      "elapsedMs": 32
    }
  }
}
```

## Crawl Start

```json
{
  "success": true,
  "id": "550e8400-e29b-41d4-a716-446655440000"
}
```

## Crawl Status

```json
{
  "success": true,
  "status": "completed",
  "total": 12,
  "completed": 12,
  "data": [
    {
      "markdown": "# Page content",
      "metadata": {
        "sourceURL": "https://example.com/page"
      }
    }
  ]
}
```

## Map

```json
{
  "success": true,
  "data": {
    "links": [
      "https://example.com",
      "https://example.com/about"
    ]
  }
}
```

## Search

**Two hosts, two shapes — read `data.results` when present, otherwise `data`.** A self-hosted engine wraps the results, as shown below. The hosted API at `api.fastcrw.com` puts the results **directly in `data`** and hoists `answer`, `citations`, `llmUsage` and `warnings` to the top level, so adding `answer: true` never changes `data`'s shape.

`answer`, `citations`, `llmUsage` and `warnings` are **omitted entirely** when they have nothing to report — they are not emitted as `null` or `[]`. A plain self-hosted search is usually just `{"success": true, "data": {"results": [...]}}`. Test with `"answer" in data`, never by truthiness of a key you assume exists.

Two of them appear without any LLM feature being used: `data.warnings` is populated when a search leg degrades, and a top-level `warning` (singular, on the envelope) appears when `scrapeOptions` enrichment fails. Both are routine partial-success signals, not errors — do not assert on the exact key set.

The self-hosted wrapper, with every optional field populated:

```json
{
  "success": true,
  "data": {
    "results": <flat array OR grouped object — see below>,
    "answer": "Synthesized answer over the top results.",
    "citations": [
      { "url": "https://...", "title": "...", "position": 0 }
    ],
    "llmUsage": { "inputTokens": 3420, "outputTokens": 96, "totalTokens": 3516, "estimatedCostUsd": 0.0008, "model": "...", "provider": "..." },
    "warnings": ["answer synthesis unavailable"]
  }
}
```

The hosted equivalent of the same response — same fields, one level higher:

```json
{
  "success": true,
  "data": <flat array OR grouped object — same two shapes as below>,
  "answer": "Synthesized answer over the top results.",
  "citations": [
    { "url": "https://...", "title": "...", "position": 0 }
  ],
  "llmUsage": { "inputTokens": 3420, "outputTokens": 96, "totalTokens": 3516 },
  "warnings": ["answer synthesis unavailable"],
  "_meta": { "...": "request accounting; present on the LLM path" }
}
```

`results` shape (identical on both hosts — only its position differs):

- flat array when `sources` is not set
- grouped object when `sources` is set

Flat (each entry may also carry `markdown` from `scrapeOptions` and `summary` from `summarizeResults`):

```json
[
  {
    "url": "https://example.com/article",
    "title": "Article Title",
    "description": "Search snippet...",
    "position": 1,
    "score": 9.5,
    "markdown": "# Article Title\n\n…",
    "summary": "Short LLM-generated digest of this single result."
  }
]
```

Grouped:

```json
{
  "web": [{ "url": "...", "title": "..." }],
  "news": [{ "url": "...", "title": "...", "publishedDate": "2026-04-02T14:00:00" }]
}
```

`answer`, `citations`, and the top-level `llmUsage` are populated only when `answer: true` was sent. Per-result `summary` is populated only when `summarizeResults: true` was sent.

## Error Envelope

```json
{
  "success": false,
  "error": "Human-readable error message",
  "error_code": "machine_readable_code"
}
```

## What To Read Next

- [Scrape](#scraping)
- [Crawl](#crawling)
- [Map](#map)
- [Search](#search)
- [Error Codes](#error-codes)
