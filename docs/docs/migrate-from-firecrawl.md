# Migrate from Firecrawl in One Afternoon

**Goal:** Move a working Firecrawl integration to fastCRW in a single afternoon. This guide covers the four mechanical changes (base URL, SDK package, env var, extraction shape) plus the behavioral differences you must validate before going to production.

**Time estimate:** 15 minutes of code changes, 30–60 minutes of validation.

---

## What transfers without changes

The core request shape is intentionally compatible. These work against fastCRW with no modification when you point the client at the right base URL:

- `/v1/scrape` — same body fields (`url`, `formats`, `onlyMainContent`, `waitFor`, etc.)
- `/v1/crawl` + `/v1/crawl/{id}` polling
- `/v1/map`
- `/v1/search`
- `Authorization: Bearer <key>` header

---

## Step 1 — Swap the base URL

Every engine call goes to `https://api.fastcrw.com` instead of `https://api.firecrawl.dev`.

:::tabs
::tab{title="cURL — before"}
```bash
curl -X POST https://api.firecrawl.dev/v1/scrape \
  -H "Authorization: Bearer $FIRECRAWL_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://news.ycombinator.com",
    "formats": ["markdown"]
  }'
```
::tab{title="cURL — after"}
```bash
curl -X POST https://api.fastcrw.com/v1/scrape \
  -H "Authorization: Bearer $CRW_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://news.ycombinator.com",
    "formats": ["markdown"]
  }'
```
:::

**Self-hosted:** if you run your own `crw-server`, replace `https://api.fastcrw.com` with your server's address (e.g. `http://crw:3000`). Authentication is optional on self-hosted; disable it with `[server] auth_required = false` in your `config.toml`.

:::note{title="Keeping the Firecrawl SDK? Just repoint the API URL"}
You don't have to swap SDKs. fastCRW mirrors Firecrawl's v2 API under a dedicated **`/firecrawl/*`** namespace, so an existing Firecrawl client works once you point its base URL at fastCRW. How you set it differs by language because the two SDKs build request URLs differently:

**TypeScript / JavaScript** (`@mendable/firecrawl-js`) — include `/firecrawl` in `apiUrl`. The SDK concatenates the path, so requests land on `/firecrawl/v2/*`:
```typescript
const app = new FirecrawlApp({
  apiKey: process.env.CRW_API_KEY,
  apiUrl: "https://api.fastcrw.com/firecrawl",
});
```

**Python** (`firecrawl-py`) — use the **bare host**, no `/firecrawl`. The Python SDK resolves endpoint paths against the host root (via `urljoin`), so a `/firecrawl` suffix is silently dropped; you land on the equivalent root `/v2/*` surface, which is the same engine:
```python
app = FirecrawlApp(
    api_key=os.environ["CRW_API_KEY"],
    api_url="https://api.fastcrw.com",  # resolves to /v2/* — the same compat surface
)
```

Both reach the same Firecrawl-compatible engine. The **v2 SDK is the supported drop-in**; the legacy v1 SDK is compatible only for `scrape`.
:::

---

## Step 2 — Swap the SDK package

### Python

:::tabs
::tab{title="Before (firecrawl-py)"}
```bash
pip install firecrawl-py
```

```python
from firecrawl import FirecrawlApp

app = FirecrawlApp(api_key="fc-...")

result = app.scrape_url(
    "https://news.ycombinator.com",
    params={"formats": ["markdown"]},
)
print(result["markdown"][:500])
```
::tab{title="After (crw)"}
```bash
pip install crw
```

```python
from crw import CrwClient

client = CrwClient()  # reads CRW_API_KEY from env

result = client.scrape(
    "https://news.ycombinator.com",
    formats=["markdown"],
)
print(result["markdown"][:500])
```
:::

### TypeScript / JavaScript

:::tabs
::tab{title="Before (@mendable/firecrawl-js)"}
```bash
npm install @mendable/firecrawl-js
```

```typescript
import FirecrawlApp from "@mendable/firecrawl-js";

const app = new FirecrawlApp({ apiKey: "fc-..." });

const result = await app.scrapeUrl("https://news.ycombinator.com", {
  formats: ["markdown"],
});
console.log(result.markdown?.slice(0, 500));
```
::tab{title="After (crw-sdk)"}
```bash
npm install crw-sdk
```

```typescript
import { CrwClient } from "crw-sdk";

const client = new CrwClient({ apiKey: process.env.CRW_API_KEY });

const result = await client.scrape("https://news.ycombinator.com", {
  formats: ["markdown"],
});
console.log(result.markdown?.slice(0, 500));
```
:::

---

## Step 3 — Rename the environment variable

| Old var | New var | Purpose |
|---|---|---|
| `FIRECRAWL_API_KEY` | `CRW_API_KEY` | Engine API key |
| `FIRECRAWL_API_URL` | `CRW_API_URL` | Custom engine URL (self-hosted) |

Shell (`.env` or export):

```bash
# remove
# export FIRECRAWL_API_KEY=fc-...

# add
export CRW_API_KEY=crw-...
```

Both SDKs read the variable automatically. No code change is needed once the env var is in place.

---

## Step 4 — Port LLM extraction

Firecrawl exposes a standalone `/v1/extract` route. fastCRW supports both styles: **single-URL** extraction runs through `/v1/scrape` with `formats: ["json"]` and a `jsonSchema` (response field `data.json`), and **multi-URL** extraction has a native async `POST /v1/extract` (URLs + prompt/schema) that returns a per-URL `results` array via `GET /v1/extract/{id}`.

Multi-URL extraction: send `{ "urls": [...], "prompt"/"schema": ... }` to `/v1/extract`, then poll `/v1/extract/{id}` for `results` (one `{ url, status, data, error }` per URL).

:::tabs
::tab{title="Python — before (Firecrawl /extract)"}
```python
from firecrawl import FirecrawlApp

app = FirecrawlApp(api_key="fc-...")

schema = {
    "type": "object",
    "properties": {
        "title": {"type": "string"},
        "price": {"type": "string"},
    },
    "required": ["title", "price"],
}

result = app.extract(
    ["https://example.com/product"],
    {"schema": schema},
)
print(result)
```
::tab{title="Python — after (crw scrape + json_schema)"}
```python
from crw import CrwClient

client = CrwClient()  # reads CRW_API_KEY from env

schema = {
    "type": "object",
    "properties": {
        "title": {"type": "string"},
        "price": {"type": "string"},
    },
    "required": ["title", "price"],
}

# Pass json_schema — the SDK adds "json" to formats automatically
result = client.scrape(
    "https://example.com/product",
    json_schema=schema,
)
print(result["json"])
```
:::

:::tabs
::tab{title="TypeScript — before (Firecrawl /extract)"}
```typescript
import FirecrawlApp from "@mendable/firecrawl-js";

const app = new FirecrawlApp({ apiKey: "fc-..." });

const schema = {
  type: "object",
  properties: {
    title: { type: "string" },
    price: { type: "string" },
  },
  required: ["title", "price"],
};

const result = await app.extract(["https://example.com/product"], {
  schema,
});
console.log(result.data);
```
::tab{title="TypeScript — after (crw-sdk scrape + jsonSchema)"}
```typescript
import { CrwClient } from "crw-sdk";

const client = new CrwClient({ apiKey: process.env.CRW_API_KEY });

const schema = {
  type: "object",
  properties: {
    title: { type: "string" },
    price: { type: "string" },
  },
  required: ["title", "price"],
};

// Pass jsonSchema — the SDK adds "json" to formats automatically
const result = await client.scrape("https://example.com/product", {
  jsonSchema: schema,
});
console.log(result.json);
```
:::

---

## Behavioral differences to validate

The following table lists the gaps documented in [COMPATIBILITY-firecrawl.md](../COMPATIBILITY-firecrawl.md). Validate each that applies to your workload **before** switching production traffic.

| Feature | Firecrawl | fastCRW | Action required |
|---|---|---|---|
| **`/v1/extract` route** | Standalone async route | Not implemented — use `/v1/scrape` + `jsonSchema` | Port single-URL extract calls (see Step 4 above). Multi-URL loops need iteration. |
| **Multi-URL `/extract`** | One call → N URLs | Not supported | Iterate URLs in your code or use `/v1/crawl`. |
| **`/v1/deep-research`** | Cloud-only Firecrawl feature | Not implemented | No equivalent path — remove or redesign. |
| **`/v1/agent` (Spark models)** | Cloud-only Firecrawl feature | Not implemented | No equivalent path. |
| **`/firecrawl/v2/parse` file types** | PDF, DOCX, XLSX, ODT, RTF | PDF only (pure-Rust `pdf-inspector`, no OCR) | If you upload non-PDF files or rely on OCR, keep Firecrawl for those calls. |
| **OCR mode on PDF** | `mode: "ocr"` supported | Accepted for wire-compat; falls back to text-layer extraction with `pdf_scanned` warning | Scanned-only PDFs won't extract text. |
| **Fire-engine anti-bot** | Firecrawl Cloud only | Not available (same as Firecrawl self-host) | For heavy bot-protected pages, compare output quality on real targets. |
| **Screenshot format** | Supported | Not supported — `/v1/scrape` rejects `"screenshot"` with HTTP 400 (`"Unknown format 'screenshot'"`) | Remove `"screenshot"` from your `formats` array. |
| **`data.metadata` field names** | Some keys differ | Minor divergence on a few keys | Inspect `metadata` on a real response; don't assume key names are identical. |
| **MCP tool names** | `firecrawl_scrape`, `firecrawl_crawl`, … | `crw_scrape`, `crw_crawl`, `crw_check_crawl_status`, `crw_map`, `crw_extract`, `crw_check_extract_status`, `crw_search`, `crw_parse_file` | Update any MCP client tool-name references. |

---

## 5-minute verification checklist

Run these five commands against a real URL from your workload after making the changes above. Use a URL you can manually inspect in your browser.

**1. Smoke test — basic scrape**

```bash
curl -s -X POST https://api.fastcrw.com/v1/scrape \
  -H "Authorization: Bearer $CRW_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"url": "https://news.ycombinator.com", "formats": ["markdown"]}' \
  | python3 -m json.tool | head -30
```

Expected: `"success": true`, `"markdown"` field with real content.

**2. Crawl smoke test**

```bash
curl -s -X POST https://api.fastcrw.com/v1/crawl \
  -H "Authorization: Bearer $CRW_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com", "maxDepth": 1, "maxPages": 3}' \
  | python3 -m json.tool
```

Expected: `{"success": true, "id": "<job-id>"}`. Poll `GET /v1/crawl/{id}` until `"status": "completed"`.

**3. Map smoke test**

```bash
curl -s -X POST https://api.fastcrw.com/v1/map \
  -H "Authorization: Bearer $CRW_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com", "maxDepth": 1}' \
  | python3 -m json.tool | head -20
```

Expected: `"links": [...]` with discovered URLs.

**4. Search smoke test**

```bash
curl -s -X POST https://api.fastcrw.com/v1/search \
  -H "Authorization: Bearer $CRW_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"query": "fastcrw web scraper", "limit": 3}' \
  | python3 -m json.tool | head -30
```

Expected: `"data": {"results": [...]}` with search results.

**5. SDK round-trip (Python)**

```python
import os
from crw import CrwClient

client = CrwClient()  # CRW_API_KEY must be set

# Basic scrape
result = client.scrape("https://example.com", formats=["markdown"])
assert result.get("markdown"), "markdown field is empty"
print("scrape: OK")

# Map
links = client.map("https://example.com", max_depth=1)
assert isinstance(links, list) and len(links) > 0, "map returned no links"
print(f"map: OK ({len(links)} links)")

print("All checks passed — safe to promote to production.")
```

---

## Feature-detect the engine

Use `GET /v1/capabilities` to confirm what the target engine supports before sending calls that depend on optional features (LLM extraction, file parsing, search):

```bash
curl -s https://api.fastcrw.com/v1/capabilities \
  -H "Authorization: Bearer $CRW_API_KEY" \
  | python3 -m json.tool
```

The response lists `llm`, `search`, `documents.parsers`, and `formats` so your code can branch instead of assuming.

---

## What is not in scope

The following Firecrawl Cloud capabilities have no equivalent in fastCRW and are not planned:

- `/v1/deep-research` (Spark model pipeline)
- `/v1/agent` (AI agent sessions)
- Fire-engine proprietary anti-bot layer
- Rotating proxy pool (bring your own proxy via the `proxy` scrape param)

For any capability not in the matrix above, check [`COMPATIBILITY-firecrawl.md`](../COMPATIBILITY-firecrawl.md) which is the authoritative reference.
