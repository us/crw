<div class="page-intro">
  <div class="page-kicker">Get Started</div>
  <h1>Quick Start</h1>
  <p class="page-subtitle">Get one successful CRW response as fast as possible. The shortest path is the hosted API on <code>fastcrw.com</code>; MCP and self-hosting come right after that.</p>
  <div class="page-capabilities">
    <div class="page-capability"><strong>Goal:</strong> first success in under a minute</div>
    <div class="page-capability"><strong>Start with:</strong> cloud API</div>
    <div class="page-capability"><strong>Then branch to:</strong> MCP or self-host</div>
  </div>
  <div class="page-actions">
    <a class="page-btn primary" href="https://fastcrw.com/register" target="_blank" rel="noopener">Get API key</a>
    <a class="page-btn secondary" href="https://fastcrw.com/playground" target="_blank" rel="noopener">Open Playground</a>
  </div>
</div>

## Start here

1. Get an API key.
2. Copy the request below.
3. Confirm you got a markdown response back.

If you reach step 3, you are unblocked for almost every other page in this docs set.

## Authentication

Create an account at [fastcrw.com/register](https://fastcrw.com/register), then send the key in the `Authorization` header:

```http
Authorization: Bearer YOUR_API_KEY
```

## First request

```bash
curl -X POST https://fastcrw.com/api/v1/scrape \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com",
    "formats": ["markdown"]
  }'
```

## First response

```json
{
  "success": true,
  "data": {
    "markdown": "# Example Domain\n\nThis domain is for use in illustrative examples...",
    "metadata": {
      "title": "Example Domain",
      "sourceURL": "https://example.com",
      "statusCode": 200,
      "elapsedMs": 32
    }
  }
}
```

That is the default CRW shape most users should start with: one URL, one markdown output, no extra knobs.

## The same request in code

:::tabs
::tab{title="cURL"}
```bash
curl -X POST https://fastcrw.com/api/v1/scrape \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"url":"https://example.com","formats":["markdown"]}'
```
::tab{title="Python"}
```python
import requests

resp = requests.post(
    "https://fastcrw.com/api/v1/scrape",
    headers={
        "Authorization": "Bearer YOUR_API_KEY",
        "Content-Type": "application/json",
    },
    json={
        "url": "https://example.com",
        "formats": ["markdown"],
    },
)

print(resp.json()["data"]["markdown"])
```
::tab{title="Node.js"}
```javascript
const resp = await fetch("https://fastcrw.com/api/v1/scrape", {
  method: "POST",
  headers: {
    "Authorization": "Bearer YOUR_API_KEY",
    "Content-Type": "application/json"
  },
  body: JSON.stringify({
    url: "https://example.com",
    formats: ["markdown"]
  })
});

const body = await resp.json();
console.log(body.data.markdown);
```
:::

## Pick the next page

:::cards
::card{icon="code" title="Need a known URL?" href="#scraping" description="Stay with scrape and add formats, selectors, JS rendering, or extraction."}
::card{icon="search" title="Need unknown URLs?" href="#search" description="Use search first, then scrape only the results you care about."}
::card{icon="map" title="Need discovery on one site?" href="#map" description="Use map when you need reachability before you recurse."}
::card{icon="globe" title="Need multiple pages?" href="#crawling" description="Use crawl for bounded recursion after you validate the target section."}
:::

## MCP path

If your real goal is to give an AI tool live web access, go straight to MCP:

```bash
claude mcp add crw -- npx crw-mcp
```

For Codex, Cursor, Windsurf, and others, continue in [MCP Server](#mcp).

## Self-host path

If you want a local or private deployment instead of the hosted API:

```bash
docker run -p 3000:3000 ghcr.io/us/crw
```

Then call the local API:

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{"url":"https://example.com","formats":["markdown"]}'
```

Use [Self-Hosting](#self-hosting) for the full deployment path and [Installation](#installation) for package-level install options.

## Common mistakes

- Using too many options in the first request. Start with `formats: ["markdown"]`.
- Turning on JS rendering before checking whether plain HTTP already works.
- Jumping into `crawl` before validating the target with `scrape`.
- Treating `search` as self-hosted. In these docs, `search` is the hosted/cloud path unless noted otherwise.

## What to read next

- [Scrape](#scraping) for the canonical single-page flow
- [Search](#search) for discovery-first workflows
- [Authentication](#authentication) for key handling
- [Self-Hosting](#self-hosting) if you want to move off the hosted path
