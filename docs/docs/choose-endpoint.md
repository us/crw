<div class="page-intro">
  <div class="page-kicker">Concepts</div>
  <h1>Choose Your Endpoint</h1>
  <p class="page-subtitle">CRW has a native /v1 API for new integrations and a /firecrawl/v2 compatibility layer for Firecrawl migrations. Pick the capability that matches your input and output.</p>
  <div class="page-capabilities">
    <div class="page-capability"><strong>Six verbs:</strong> scrape, map, crawl, search, extract, parse</div>
    <div class="page-capability"><strong>Start here:</strong> native <code>/v1</code></div>
    <div class="page-capability"><strong>Extract note:</strong> not a separate route — it is scrape + JSON format</div>
  </div>
  <div class="page-actions">
    <a class="page-btn primary" href="#decision-tree">Jump to decision tree</a>
    <a class="page-btn secondary" href="#comparison-table">View comparison table</a>
  </div>
</div>

## Comparison table

New to CRW? Use `/v1`. Use `/firecrawl/v2` when migrating Firecrawl v2 SDK code or when the feature only exists on the compatibility surface, such as `POST /firecrawl/v2/batch/scrape` or `POST /firecrawl/v2/parse`.

| Verb | Route | Input | Output | Use when | LLM required? |
|---|---|---|---|---|---|
| **scrape** | `POST /v1/scrape` | A single URL | Markdown, HTML, plain text, links, or raw HTML | You know the exact page URL and want its content | No (yes for `summary` format) |
| **map** | `POST /v1/map` | A domain or start URL | List of URLs discovered under that origin | You need to enumerate pages before scraping or crawling | No |
| **crawl** | `POST /v1/crawl` | A start URL | Async job — poll `GET /v1/crawl/{id}` for all pages | You want every page under a URL scraped in one background job | No (yes if you add `summary` to `scrapeOptions`) |
| **search** | `POST /v1/search` | A query string | Ranked web search results, optionally with scraped content | You do not have a URL — you want the web to find relevant pages | No (yes for `answer`/`summarize_results` options) |
| **extract** | `POST /v1/scrape` | A URL + JSON schema | `data.json` — a filled-in object matching your schema | You need structured fields (price, title, date…) not prose | **Yes** |
| **parse** | `POST /firecrawl/v2/parse` | A PDF file upload | Markdown (or JSON/summary with schema) from the document | You have a local file, not a URL | No (yes for `summary`/`json` formats) |

> **Extract is not a separate route.** It is the same `POST /v1/scrape` endpoint with
> `formats: ["json"]` and a `jsonSchema` field. The engine scrapes the page, then passes
> the content to an LLM alongside your schema to fill it in.

## Decision tree

```
Do you have a file (PDF) to parse?
  └─ Yes ──► Parse   POST /firecrawl/v2/parse

Do you know the exact URL of the page you want?
  ├─ Yes ──► Do you need structured fields (price, date, …)?
  │            ├─ Yes ──► Extract  POST /v1/scrape  (formats:["json"] + jsonSchema)
  │            └─ No  ──► Scrape   POST /v1/scrape
  └─ No  ──► Are you looking across an entire site?
               ├─ Yes ──► Do you want every page's content in one job?
               │            ├─ Yes ──► Crawl  POST /v1/crawl
               │            └─ No  ──► Map    POST /v1/map
               └─ No  ──► Search  POST /v1/search
```

## About extract

Extract reuses the scrape route. The only difference from a plain scrape is that you add
two fields to the request body:

```json
{
  "url": "https://example.com/product/42",
  "formats": ["json"],
  "jsonSchema": {
    "type": "object",
    "properties": {
      "title":  { "type": "string" },
      "price":  { "type": "string" }
    },
    "required": ["title"]
  }
}
```

CRW scrapes the page and then calls an LLM with your schema to produce the `data.json`
field in the response. Because an LLM call is involved, extraction requires either a
server-side `[extraction.llm]` configuration (self-hosted) or a per-request `llmApiKey`.
On the hosted service at `api.fastcrw.com` this is handled automatically.

See [Extract](extract.md) for the full parameter reference and provider options.

## Quick examples

**Scrape one page as markdown:**

```bash
curl -X POST https://api.fastcrw.com/v1/scrape \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"url":"https://example.com","formats":["markdown"]}'
```

**Discover all URLs on a site:**

```bash
curl -X POST https://api.fastcrw.com/v1/map \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"url":"https://example.com"}'
```

**Crawl an entire site (start + poll):**

```bash
# Start
curl -X POST https://api.fastcrw.com/v1/crawl \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"url":"https://example.com"}'

# Poll with the returned job ID
curl https://api.fastcrw.com/v1/crawl/{id} \
  -H "Authorization: Bearer YOUR_API_KEY"
```

**Search the web:**

```bash
curl -X POST https://api.fastcrw.com/v1/search \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"query":"open source web scraping 2025","limit":5}'
```

**Extract structured data:**

```bash
curl -X POST https://api.fastcrw.com/v1/scrape \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com/product/42",
    "formats": ["json"],
    "jsonSchema": {
      "type": "object",
      "properties": {
        "title": {"type": "string"},
        "price": {"type": "string"}
      },
      "required": ["title"]
    }
  }'
```

**Parse a PDF:**

```bash
curl -X POST https://api.fastcrw.com/firecrawl/v2/parse \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -F "file=@/path/to/document.pdf"
```

## What to read next

- [Scraping](scraping.md) — full scrape parameter reference
- [Map](map.md) — URL discovery and filtering
- [Crawling](crawling.md) — async crawl job management
- [Search](search.md) — query options and result enrichment
- [Extract](extract.md) — JSON schema extraction
- [Output formats](output-formats.md) — all eight `formats` values
