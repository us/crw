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
    "chunks": [
      {
        "content": "string",
        "score": 0.91,
        "index": 0
      }
    ],
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

Hosted search can return two shapes:

- flat array when `sources` is not set
- grouped object when `sources` is set

Flat:

```json
{
  "success": true,
  "data": [
    {
      "url": "https://example.com/article",
      "title": "Article Title",
      "description": "Search snippet...",
      "position": 1,
      "score": 9.5
    }
  ]
}
```

Grouped:

```json
{
  "success": true,
  "data": {
    "web": [{ "url": "...", "title": "..." }],
    "news": [{ "url": "...", "title": "...", "publishedDate": "2026-04-02T14:00:00" }]
  }
}
```

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
