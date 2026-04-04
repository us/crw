# Search Endpoint Guide

## Overview

Use `search` when you need to find content across the web without knowing specific URLs upfront. It is the right choice for:

- research workflows where the agent discovers relevant pages,
- RAG pipelines that need fresh web content on a topic,
- news monitoring and trend tracking,
- and competitive analysis across multiple sources.

```bash
curl -X POST http://localhost:3002/api/v1/search \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -d '{"query":"web scraping tools","limit":5}'
```

## A Good Default Request

If you are not sure where to start, use this shape first:

```json
{
  "query": "your search terms here",
  "limit": 5
}
```

That gives you the top 5 web results with title, URL, description, and relevance score. Add `scrapeOptions` when you need the actual page content, not just search snippets.

## Parameters

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `query` | `string` | *required* | The search query (max 2000 characters) |
| `limit` | `number` | `5` | Maximum number of results per source (1-20) |
| `lang` | `string` | -- | Language code for results (e.g. `"en"`, `"de"`, `"fr"`) |
| `tbs` | `string` | -- | Time-based filter: `"qdr:h"` (hour), `"qdr:d"` (day), `"qdr:w"` (week), `"qdr:m"` (month), `"qdr:y"` (year) |
| `sources` | `string[]` | -- | Result types to return: `"web"`, `"news"`, `"images"`. When set, response is grouped by source |
| `categories` | `string[]` | -- | Filter by category: `"github"`, `"research"`, `"pdf"` |
| `scrapeOptions` | `object` | -- | Scrape each result URL. See below |

### scrapeOptions

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `formats` | `string[]` | *required* | Output formats: `"markdown"`, `"html"`, `"rawHtml"`, `"links"` |
| `onlyMainContent` | `boolean` | `true` | Extract primary content area only |

## Response Format

### Flat response (default -- no `sources`)

When `sources` is not set, results are returned as a flat array sorted by relevance:

```json
{
  "success": true,
  "data": [
    {
      "url": "https://example.com/article",
      "title": "Article Title",
      "description": "A snippet from the search result...",
      "position": 1,
      "score": 9.5,
      "category": "general"
    }
  ]
}
```

### Grouped response (with `sources`)

When `sources` is set, results are grouped by type. The `limit` applies per source:

```json
{
  "success": true,
  "data": {
    "web": [
      { "url": "...", "title": "...", "description": "...", "position": 1, "score": 9.5 }
    ],
    "news": [
      { "url": "...", "title": "...", "description": "...", "position": 1, "publishedDate": "2026-04-02T14:00:00" }
    ]
  }
}
```

## Search + Scrape

The real power of the search endpoint is combining search with content scraping in a single call. Add `scrapeOptions` to fetch the full page content for each result:

```bash
curl -X POST http://localhost:3002/api/v1/search \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -d '{
    "query": "machine learning transformers",
    "limit": 3,
    "scrapeOptions": {
      "formats": ["markdown"]
    }
  }'
```

Each result in the response will include the scraped content:

```json
{
  "url": "https://example.com/article",
  "title": "Understanding Transformers",
  "description": "Search snippet...",
  "position": 1,
  "markdown": "# Understanding Transformers\n\nThe transformer architecture...",
  "metadata": { "statusCode": 200 }
}
```

If a particular URL fails to scrape (anti-bot, timeout), the result is still returned with the search metadata but without the scraped content. The credit for that failed scrape is refunded.

:::note
When `scrapeOptions` is combined with `sources`, only `web` results are scraped. News and image results return search metadata only.
:::

## News Search

Search specifically for recent news articles:

```bash
curl -X POST http://localhost:3002/api/v1/search \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -d '{"query":"artificial intelligence","sources":["news"],"limit":5}'
```

News results include a `publishedDate` field with the article publication timestamp.

## Image Search

Search for images across the web:

```bash
curl -X POST http://localhost:3002/api/v1/search \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -d '{"query":"neural network diagram","sources":["images"],"limit":5}'
```

Image results include `imageUrl`, `thumbnailUrl`, `imageFormat`, and `resolution` fields.

## Time-Based Search

Filter results by recency using the `tbs` parameter:

| Value | Meaning |
| --- | --- |
| `qdr:h` | Past hour |
| `qdr:d` | Past 24 hours |
| `qdr:w` | Past week |
| `qdr:m` | Past month |
| `qdr:y` | Past year |

```json
{
  "query": "latest AI announcements",
  "tbs": "qdr:d",
  "limit": 5
}
```

:::note
`qdr:h` maps to day-level precision due to backend limitations. Hourly granularity is not available.
:::

## Category Filtering

Focus your search on specific content categories:

- `"github"` -- search within GitHub repositories, code, and issues
- `"research"` -- search academic sources (arXiv, Google Scholar, Semantic Scholar)
- `"pdf"` -- search for PDF documents

```json
{
  "query": "web scraping python",
  "categories": ["github"],
  "limit": 5
}
```

Categories can be combined: `"categories": ["github", "research"]`.

## Credit Cost

:::note
Cloud only (fastcrw.com) -- self-hosted instances do not have credit-based billing.
:::

| Operation | Cost |
| --- | --- |
| Search (without scraping) | 1 credit |
| Search + scrape | 1 credit + 1 per scraped result |

If you search with `limit: 5` and `scrapeOptions`, and all 5 results scrape successfully, the total cost is 6 credits (1 search + 5 scrapes). Failed scrapes are refunded.

## Common Mistakes

- **Empty query** -- the `query` field is required and must be at least 1 character.
- **Too many results** -- `limit` caps at 20. Start with 5 and increase only if needed.
- **Scraping everything** -- adding `scrapeOptions` multiplies the credit cost. Only use it when you actually need the page content, not just search snippets.
- **Mixing sources and categories** -- `sources` controls the *type* of results (web, news, images). `categories` controls the *domain* filter (github, research). They work independently.

## What to Read Next

- [Scrape endpoint](/docs/scraping) -- for scraping specific URLs you already know.
- [Extract endpoint](/docs/extract) -- for structured JSON extraction from pages.
- [Credit costs](/docs/credit-costs) -- full billing breakdown across all endpoints.
- [SDK examples](/docs/sdk-examples) -- search examples in TypeScript, Python, and Go.
