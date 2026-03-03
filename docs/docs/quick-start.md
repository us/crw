# Quick Start

## Start the server

```bash
crw-server
```

The server starts on `http://localhost:3000` by default.

## Scrape a page

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'
```

Response:

```json
{
  "success": true,
  "data": {
    "markdown": "# Example Domain\n\nThis domain is for use in illustrative examples...",
    "metadata": {
      "title": "Example Domain",
      "description": "...",
      "language": "en",
      "sourceURL": "https://example.com"
    }
  }
}
```

## Scrape with options

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com",
    "formats": ["markdown", "links"],
    "onlyMainContent": true,
    "includeTags": ["article", "main"],
    "excludeTags": [".sidebar", ".ads"]
  }'
```

## Crawl a site

```bash
# Start a crawl job (async)
curl -X POST http://localhost:3000/v1/crawl \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com",
    "maxDepth": 2,
    "maxPages": 50
  }'
```

Response:

```json
{
  "success": true,
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "url": "http://localhost:3000/v1/crawl/550e8400-e29b-41d4-a716-446655440000"
}
```

```bash
# Check crawl status and results
curl http://localhost:3000/v1/crawl/550e8400-e29b-41d4-a716-446655440000
```

## Map a site

Discover all URLs without scraping content:

```bash
curl -X POST http://localhost:3000/v1/map \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com",
    "useSitemap": true
  }'
```

## With authentication

If `auth.api_keys` is configured, include the Bearer token:

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Authorization: Bearer fc-your-api-key" \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'
```
