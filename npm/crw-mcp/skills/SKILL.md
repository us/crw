---
name: crw
description: "Scrape, crawl, map, and search the web using fastCRW. Use when the user needs web page content, site-wide extraction, URL discovery, or web search results. Single binary, 6 MB RAM, Firecrawl-compatible API."
license: AGPL-3.0
metadata:
  author: us
  version: "0.3.0"
  homepage: https://fastcrw.com
  repository: https://github.com/us/crw
allowed-tools: Bash(npx:crw-mcp*) Bash(curl:*) Read
---

# fastCRW — Web Data Toolkit for AI Agents

## When to use this skill

Use this skill when:
- The user asks you to read, scrape, or fetch a web page
- You need to extract content from a URL for context or research
- The user wants to crawl an entire website or discover its pages
- You need to search the web and get page content (cloud mode)
- The user mentions Firecrawl — CRW is a drop-in replacement

## Installation

```bash
npx crw-mcp@latest init
```

This installs the CRW MCP server to all detected AI agents (Claude Code, Cursor, Gemini CLI, Codex, OpenCode, Windsurf, Roo Code).

## Authentication

- **Embedded mode** (default): No key needed — the MCP server runs a self-contained scraper in ~6 MB RAM. No server required.
- **Cloud mode** (fastcrw.com): Set `CRW_API_KEY=crw_live_...` and `CRW_API_URL=https://fastcrw.com/api`. Get a free key at https://fastcrw.com with 500 credits/month.

## MCP Tools

### crw_scrape

Scrape a single URL and return clean content.

Parameters:
- `url` (required) — The URL to scrape
- `formats` — Output formats: `markdown` (default), `html`, `rawHtml`, `plainText`, `links`, `json`
- `onlyMainContent` — Strip navs/footers/sidebars. Default: `true`
- `renderJs` — Force JavaScript rendering. Default: auto-detect (null)
- `cssSelector` — Extract only elements matching this CSS selector
- `xpath` — Extract only elements matching this XPath
- `includeTags` — Only include these HTML tags (e.g. `["article", "main"]`)
- `excludeTags` — Remove these HTML tags (e.g. `["nav", "footer"]`)

### crw_crawl

Start an async BFS crawl from a URL. Returns a job ID — poll with `crw_check_crawl_status`.

Parameters:
- `url` (required) — Starting URL
- `maxDepth` — Maximum link depth. Default: `2`, max: `10`
- `limit` — Maximum pages to crawl. Default: `10`, max: `1000`

Returns: `{ "id": "job-uuid" }` — use this ID with crw_check_crawl_status.

### crw_check_crawl_status

Poll an async crawl job for results.

Parameters:
- `id` (required) — The crawl job ID from `crw_crawl`

Returns: `{ "status": "pending|running|completed|failed", "data": [...] }`

### crw_map

Discover all URLs on a website via sitemap + link extraction, without scraping content.

Parameters:
- `url` (required) — The URL to map
- `maxDepth` — Discovery depth. Default: `2`
- `useSitemap` — Check sitemap.xml. Default: `true`

Returns: `{ "links": ["url1", "url2", ...] }` — up to 5000 URLs.

## Common Patterns

**Scrape a page for context:**
```
crw_scrape(url="https://example.com", formats=["markdown"])
```

**Crawl docs for RAG:**
First discover URLs, then crawl:
```
crw_map(url="https://docs.example.com")  → get URL list
crw_crawl(url="https://docs.example.com", limit=50)  → extract all content
crw_check_crawl_status(id="...")  → poll until completed
```

**Search the web (cloud mode only):**
Use the REST API directly — `POST /v1/search` with `{"query": "...", "limit": 5}`. Requires `CRW_API_URL` and `CRW_API_KEY`.

## Common Edge Cases

- **JavaScript-heavy sites**: Set `renderJs: true` if the page is blank or returns a loading skeleton
- **Rate limiting**: Cloud mode has per-plan rate limits. Check response headers for `X-RateLimit-*`
- **Large crawls**: Use `crw_map` first to estimate site size before committing to a large `crw_crawl`
- **Timeout**: Crawl jobs expire after 1 hour. Poll `crw_check_crawl_status` regularly

## Links

- Cloud API: https://fastcrw.com — 500 free credits/month
- Docs: https://docs.fastcrw.com
- GitHub: https://github.com/us/crw
- Firecrawl-compatible: same REST endpoints at `/v1/scrape`, `/v1/crawl`, `/v1/map`, `/v1/search`
