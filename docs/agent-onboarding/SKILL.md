---
name: crw
description: "Scrape, crawl, map, and search the web using fastCRW's native /v1 API. Use when the user needs web page content, site-wide extraction, URL discovery, or web search results. Single binary, 6 MB RAM; /v2 exists separately for Firecrawl migration."
license: AGPL-3.0
metadata:
  author: us
  version: "0.3.0"
  homepage: https://fastcrw.com
  repository: https://github.com/us/crw
allowed-tools: Bash(crw:*) Bash(curl:*) Read
---

# fastCRW — Web Data Toolkit for AI Agents

## When to use this skill

Use this skill when:
- The user asks you to read, scrape, or fetch a web page
- You need to extract content from a URL for context or research
- The user wants to crawl an entire website or discover its pages
- You need to search the web and get page content (cloud mode)
- The user mentions Firecrawl — use native `/v1` for new CRW work; use `/v2` only when migrating existing Firecrawl v2 SDK code

## Installation

```bash
npx crw-mcp@latest install   # installs the skill + MCP server into detected agents
npx crw-mcp@latest init      # skill only
```

`install` sets up the CRW skill and MCP server for all detected AI agents (Claude Code, Cursor, Gemini CLI, Codex, OpenCode, Windsurf, Roo Code).

## Authentication

- **Embedded mode** (default): No key needed — the MCP server runs a self-contained scraper in ~6 MB RAM. No server required.
- **Cloud mode** (fastcrw.com): Set `CRW_API_KEY=crw_live_...` and `CRW_API_URL=https://api.fastcrw.com`. Get a free key at https://fastcrw.com with 500 one-time lifetime credits (never resets, not monthly).

## MCP Tools

> **Output bounds:** By default, content is truncated to ~15 000 chars (`crw_scrape`, `crw_check_crawl_status`, `crw_parse_file`) and `crw_map` returns ≤ 100 URLs. Truncated results carry a `truncated: true` marker (`crw_map` also adds `totalDiscovered`). Pass `maxLength: 0` or `limit: 0` to opt out of bounding.

### crw_scrape

Scrape a single URL and return clean content.

Parameters:
- `url` (required) — The URL to scrape
- `formats` — Output formats: `markdown` (default), `html`, `links`
- `onlyMainContent` — Strip navs/footers/sidebars. Default: `true`
- `includeTags` — Only include content matching these CSS selectors (e.g. `["article", "main"]`)
- `excludeTags` — Exclude content matching these CSS selectors (e.g. `["nav", "footer"]`)
- `renderJs` — Force JavaScript rendering. Default: auto-detect (null)
- `waitFor` — Milliseconds to wait after page load before capturing
- `renderer` — Renderer override (e.g. `"playwright"`)
- `maxLength` — Truncate output to this many chars. `0` = unbounded. Default: ~15 000

### crw_crawl

Start an async BFS crawl from a URL. Returns a job ID — poll with `crw_check_crawl_status`.

Parameters:
- `url` (required) — Starting URL
- `maxDepth` — Maximum link depth. Default: `2`
- `maxPages` — Maximum pages to crawl
- `jsonSchema` — JSON schema for structured extraction per page
- `renderJs` — Force JavaScript rendering
- `waitFor` — Milliseconds to wait after page load before capturing
- `renderer` — Renderer override

Returns: `{ "id": "job-uuid" }` — use this ID with crw_check_crawl_status.

### crw_check_crawl_status

Poll an async crawl job for results.

Parameters:
- `id` (required) — The crawl job ID from `crw_crawl`
- `maxLength` — Truncate each page's content fields to this many chars. `0` = unbounded. Default: ~15 000

Returns: `{ "status": "scraping|completed|failed", "data": [...] }`

> **Browser Automation:** Full interactive browser control (JavaScript rendering, click, fill, etc.) requires the separate **crw-browse** MCP server binary (`command: crw-browse`). It exposes its own tools (`goto`, `tree`, and others) and is not part of this MCP server. Do not call `crw_browse` here — it is not a tool in crw-mcp and will return a JSON-RPC -32602 "Unknown tool" error.

### crw_search

Search the web for current information, news, facts, or docs. Use whenever the answer may depend on up-to-date or external information. Returns ranked results (url/title/description/snippet); optionally scrape each result inline. Always available in proxy/cloud mode; in embedded mode only when a search backend is configured.

Parameters:
- `query` (required) — The search query
- `limit` — Maximum number of results to return. Default: `5`
- `lang` — Language code for results (e.g. `"en"`, `"tr"`)
- `tbs` — Time filter: `qdr:h|qdr:d|qdr:w|qdr:m|qdr:y` (past hour/day/week/month/year)
- `sources` — If set, group results by source: `web`, `news`, `images`
- `categories` — Category bias; e.g. `"pdf"`, `"github"`, `"research"`, `"news"`, `"images"`
- `scrapeOptions` — Options for scraping each result page (e.g. `{"formats": ["markdown"]}`)

### crw_map

Discover all URLs on a website via sitemap + link extraction, without scraping content.

Parameters:
- `url` (required) — The URL to map
- `maxDepth` — Discovery depth. Default: `2`
- `useSitemap` — Check sitemap.xml. Default: `true`
- `crawlFallback` — Supplement sitemap discovery with a short BFS crawl. Default: `true` (`false` = sitemap-only)
- `limit` — Maximum URLs to return. `0` = unbounded. Default: `100`

Returns: `{ "links": ["url1", "url2", ...] }`

### crw_extract

Extract structured JSON from one or more URLs via a prompt and/or JSON schema. Async job, poll with `crw_check_extract_status`. Needs an LLM.

Parameters:
- `urls` (required) — URLs to extract from
- `prompt` — Free-text extraction objective (required unless `schema` is given)
- `schema` — JSON schema constraining the extracted output
- `llmApiKey` — BYOK LLM API key
- `llmProvider` — LLM provider (used with `llmApiKey`)
- `llmModel` — LLM model (used with `llmApiKey`)

Returns: `{ "id": "job-uuid" }` — use this ID with crw_check_extract_status.

### crw_check_extract_status

Poll an async extract job for results.

Parameters:
- `id` (required) — The extract job ID from `crw_extract`

Returns: status and, when complete, a per-URL results array.

### crw_cancel_extract

Idempotently request cancellation of an extract job. A claimed URL may finish
while status is `cancelling`; terminal `cancelled` preserves that result and
marks every untouched ordered slot `cancelled`.

Parameters:
- `id` (required) — The extract job ID from `crw_extract`

Returns the same canonical status envelope as `crw_check_extract_status`.

### crw_parse_file

Parse a local file (PDF) into markdown or structured output without fetching from the web.

Parameters:
- `contentBase64` (required) — Base64-encoded file contents
- `filename` — Original filename (optional, e.g. `"report.pdf"`)
- `formats` — Output formats: `markdown` (default), `plainText`, `links`, `json`, `summary` (json/summary need a server LLM)
- `jsonSchema` — JSON schema for LLM extraction (when `formats` includes `json`)
- `parsers` — Document parsers to apply. Default: `["pdf"]`
- `maxLength` — Truncate output to this many chars. `0` = unbounded. Default: ~15 000

## Common Patterns

**Scrape a page for context:**
```
crw_scrape(url="https://example.com", formats=["markdown"])
```

**Crawl docs for RAG:**
First discover URLs, then crawl:
```
crw_map(url="https://docs.example.com")  → get URL list
crw_crawl(url="https://docs.example.com", maxPages=50)  → extract all content
crw_check_crawl_status(id="...")  → poll until completed
```

**Search the web:**
```
crw_search(query="your search query", limit=5)
```

**Search from the CLI (one-shot LLM-ready output):**

When the `crw` binary is available, prefer the native field projection
over piping through `jq` — it's one call instead of two:

```bash
crw search "renewable energy 2024" --json --fields title,url,snippet --limit 3
```

Available fields: `title`, `url`, `description`, `snippet`, `position`,
`score`, `category`. `--json` is shorthand for `--format json`.

## Common Edge Cases

- **JavaScript-heavy sites**: Set `renderJs: true` if the page is blank or returns a loading skeleton
- **Rate limiting**: Cloud mode has per-plan rate limits. Check response headers for `X-RateLimit-*`
- **Large crawls**: Use `crw_map` first to estimate site size before committing to a large `crw_crawl`
- **Timeout**: Crawl jobs expire after 1 hour. Poll `crw_check_crawl_status` regularly

## Links

- Cloud API: https://fastcrw.com — 500 one-time lifetime free credits (never resets, not monthly)
- Docs: https://docs.fastcrw.com
- GitHub: https://github.com/us/crw
- Native API: `/v1/scrape`, `/v1/crawl`, `/v1/map`, and `/v1/search` are the recommended routes for new CRW integrations
- Firecrawl migration: `/v2/*` is a compatibility layer, not the default API for new builds
