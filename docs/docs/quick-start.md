# Quick Start — Web Scraping with CRW

## Install

```bash
# One-line install (auto-detects OS & arch):
curl -fsSL https://raw.githubusercontent.com/us/crw/main/install.sh | sh
```

See all installation options in the [installation guide](installation.md).

## CLI (no server needed)

The `crw` binary lets you scrape any URL directly from the terminal — no server, no config file, no setup:

```bash
# Markdown to stdout (default)
crw https://example.com

# JSON output
crw https://example.com --format json

# Save to file
crw https://example.com -o page.md

# Plain text
crw https://example.com --format text

# Extract all links
crw https://example.com --format links

# Keep full page (skip main-content extraction)
crw https://example.com --raw

# Narrow with CSS selector
crw https://example.com --css 'article.content'
```

---

## REST API server

```bash
crw-server
```

The cloud API is at `https://fastcrw.com/api`. For self-hosted, the server starts on `https://fastcrw.com/api` by default.

## Scrape a page

```bash
curl -X POST https://fastcrw.com/api/v1/scrape \
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
curl -X POST https://fastcrw.com/api/v1/scrape \
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
curl -X POST https://fastcrw.com/api/v1/crawl \
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
  "url": "https://fastcrw.com/api/v1/crawl/550e8400-e29b-41d4-a716-446655440000"
}
```

```bash
# Check crawl status and results
curl https://fastcrw.com/api/v1/crawl/550e8400-e29b-41d4-a716-446655440000
```

## Map a site

Discover all URLs without scraping content:

```bash
curl -X POST https://fastcrw.com/api/v1/map \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com",
    "useSitemap": true
  }'
```

## With authentication

If `auth.api_keys` is configured, include the Bearer token:

```bash
curl -X POST https://fastcrw.com/api/v1/scrape \
  -H "Authorization: Bearer fc-your-api-key" \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'
```
