# Recipe: Batch-Scrape a URL List

Scrape dozens (or hundreds) of unrelated URLs in one async job — no crawling, no link discovery, just your exact list processed in parallel.

---

## When to Use Batch Instead of Crawl

| Situation | Use |
|---|---|
| You already know every URL you want | **Batch** |
| You need to discover links from a seed domain | Crawl |
| URLs span multiple domains | **Batch** |
| You want `maxDepth` / `maxPages` control over discovery | Crawl |
| You want all pages under one site section | Crawl |
| Processing a CSV export, sitemap, or search results list | **Batch** |

Batch (`POST /firecrawl/v2/batch/scrape`) and crawl (`POST /firecrawl/v2/crawl`) share the same async job machinery and identical status/response envelopes. The difference is the input: batch takes an explicit `urls` array, crawl takes a single seed URL and discovers the rest itself.

> **Native vs Firecrawl-compat.** This recipe uses the Firecrawl-compatible `/firecrawl/v2/batch/scrape` surface. There is also a native twin at `POST /v1/batch/scrape` (+ `GET`/`DELETE /v1/batch/scrape/{id}`) that follows the native v1 conventions: strict camelCase, so the flag is spelled `ignoreInvalidUrls` (not the v2 `ignoreInvalidURLs`) and the rejected list is returned as `invalidUrls` (not `invalidURLs`), and the status envelope matches `GET /v1/crawl/{id}`. Prefer `/v1` for new native integrations; keep `/firecrawl/v2` only when reusing Firecrawl SDK payloads verbatim.

---

## How It Works

```
POST /firecrawl/v2/batch/scrape        →  { id, url, invalidURLs }
GET  /firecrawl/v2/batch/scrape/{id}   →  { status, total, completed, data[], next }
GET  /firecrawl/v2/batch/scrape/{id}?skip=100   (paginate large results)
DELETE /firecrawl/v2/batch/scrape/{id}  (cancel)
GET  /firecrawl/v2/batch/scrape/{id}/errors
```

**Status values:** `scraping` → `completed` | `failed` | `cancelled`

The response is paginated (100 documents per page, max ~10 MB per page). While `status` is `scraping`, the `next` cursor is set even if the current page is empty — keep polling forward until `next` is `null` and `status` is `completed`.

---

## Examples

### Target URLs

These three unrelated pages are used throughout the examples below:

```
https://news.ycombinator.com/
https://github.com/trending
https://lobste.rs/
```

---

### cURL

**Step 1 — Start the job**

```bash
curl -s -X POST https://api.fastcrw.com/firecrawl/v2/batch/scrape \
  -H "Authorization: Bearer $CRW_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "urls": [
      "https://news.ycombinator.com/",
      "https://github.com/trending",
      "https://lobste.rs/"
    ],
    "formats": ["markdown", "links"]
  }'
```

Expected response:

```json
{
  "success": true,
  "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "url": "https://api.fastcrw.com/firecrawl/v2/batch/scrape/a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "invalidURLs": []
}
```

Save the `id` value.

---

**Step 2 — Poll for status**

```bash
JOB_ID="a1b2c3d4-e5f6-7890-abcd-ef1234567890"

curl -s "https://api.fastcrw.com/firecrawl/v2/batch/scrape/$JOB_ID" \
  -H "Authorization: Bearer $CRW_API_KEY"
```

Repeat every 2–3 seconds until `"status": "completed"`. While still running:

```json
{
  "success": true,
  "status": "scraping",
  "total": 3,
  "completed": 1,
  "creditsUsed": 1,
  "expiresAt": "2026-06-16T10:00:00.000Z",
  "next": "https://api.fastcrw.com/firecrawl/v2/batch/scrape/a1b2c3d4-e5f6-7890-abcd-ef1234567890?skip=0",
  "data": [
    {
      "markdown": "# Hacker News\n...",
      "links": ["https://news.ycombinator.com/item?id=..."],
      "metadata": {
        "title": "Hacker News",
        "sourceURL": "https://news.ycombinator.com/",
        "url": "https://news.ycombinator.com/",
        "statusCode": 200,
        "proxyUsed": "basic",
        "cacheState": "miss",
        "concurrencyLimited": false,
        "creditsUsed": 1,
        "scrapeId": "f1a2b3c4-..."
      }
    }
  ]
}
```

When complete:

```json
{
  "success": true,
  "status": "completed",
  "total": 3,
  "completed": 3,
  "creditsUsed": 3,
  "expiresAt": "2026-06-16T10:00:00.000Z",
  "next": null,
  "data": [ /* all 3 documents */ ]
}
```

---

**Step 3 — Paginate large results (optional)**

For jobs with many URLs, use `?skip=N` to page through results. `next` always contains the ready-to-use URL:

```bash
# Page 2 (skip the first 100 docs)
curl -s "https://api.fastcrw.com/firecrawl/v2/batch/scrape/$JOB_ID?skip=100" \
  -H "Authorization: Bearer $CRW_API_KEY"
```

Follow `next` until it is `null`.

---

### Python

The `crw` SDK ships `batch_scrape()` which handles starting, polling, and paginating internally. It returns the flat list of page-result dicts once the job completes.

```python
import os
from crw import CrwClient

client = CrwClient(api_key=os.environ["CRW_API_KEY"])

urls = [
    "https://news.ycombinator.com/",
    "https://github.com/trending",
    "https://lobste.rs/",
]

# Start the job, poll until done, collect all results.
# poll_interval: seconds between status checks (default 2.0)
# timeout: max total wait in seconds (default 300.0)
pages = client.batch_scrape(
    urls,
    formats=["markdown", "links"],
    poll_interval=2.0,
    timeout=120.0,
)

for page in pages:
    meta = page.get("metadata", {})
    print(f"URL: {meta.get('sourceURL')}")
    print(f"Status: {meta.get('statusCode')}")
    md = page.get("markdown", "")
    print(f"Content ({len(md)} chars): {md[:200]}")
    print("---")
```

Expected output:

```
URL: https://news.ycombinator.com/
Status: 200
Content (4821 chars): # Hacker News

Ask HN: ... | 312 points | 143 comments ...
---
URL: https://github.com/trending
Status: 200
Content (5102 chars): # Trending repositories on GitHub today ...
---
URL: https://lobste.rs/
Status: 200
Content (3874 chars): # Lobsters

[Show HN] ... submitted 2 hours ago ...
---
```

---

#### Raw HTTP (no SDK) — Python

Use this when you want full control or need to integrate batch scraping into an existing HTTP session:

```python
import os
import time
import urllib.request
import json

API_KEY = os.environ["CRW_API_KEY"]
BASE = "https://api.fastcrw.com"

def _call(method: str, path: str, body: dict | None = None) -> dict:
    url = f"{BASE}{path}"
    data = json.dumps(body).encode() if body else None
    req = urllib.request.Request(
        url, data=data,
        headers={
            "Content-Type": "application/json",
            "Authorization": f"Bearer {API_KEY}",
        },
        method=method,
    )
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.loads(r.read())

# 1. Start batch job
start = _call("POST", "/firecrawl/v2/batch/scrape", {
    "urls": [
        "https://news.ycombinator.com/",
        "https://github.com/trending",
        "https://lobste.rs/",
    ],
    "formats": ["markdown", "links"],
})
job_id = start["id"]
print(f"Job started: {job_id}")

# 2. Poll until completed
while True:
    status = _call("GET", f"/firecrawl/v2/batch/scrape/{job_id}")
    print(f"  {status['completed']}/{status['total']} completed ({status['status']})")
    if status["status"] == "completed":
        break
    if status["status"] == "failed":
        raise RuntimeError(f"Batch failed: {status.get('error')}")
    time.sleep(2)

# 3. Collect all pages (follow `next` cursor for large jobs)
all_docs = []
skip = 0
while True:
    page = _call("GET", f"/firecrawl/v2/batch/scrape/{job_id}?skip={skip}")
    all_docs.extend(page["data"])
    next_url = page.get("next")
    if not next_url or page["status"] != "completed":
        break
    # parse skip from the next URL
    skip = int(next_url.split("skip=")[-1])

print(f"\nCollected {len(all_docs)} documents")
for doc in all_docs:
    src = doc["metadata"]["sourceURL"]
    chars = len(doc.get("markdown") or "")
    print(f"  {src}: {chars} chars of markdown")
```

Expected output:

```
Job started: a1b2c3d4-e5f6-7890-abcd-ef1234567890
  1/3 completed (scraping)
  2/3 completed (scraping)
  3/3 completed (completed)

Collected 3 documents
  https://news.ycombinator.com/: 4821 chars of markdown
  https://github.com/trending: 5102 chars of markdown
  https://lobste.rs/: 3874 chars of markdown
```

---

## Key Request Fields

All fields except `urls` are optional.

| Field | Type | Default | Notes |
|---|---|---|---|
| `urls` | `string[]` | required | At least one valid URL |
| `formats` | `string[]` | `["markdown"]` | Any of: `markdown`, `html`, `rawHtml`, `plainText`, `links`, `json`, `summary`, `changeTracking` |
| `onlyMainContent` | `bool` | `true` | Strip nav/footer boilerplate |
| `waitFor` | `number` | — | MS to wait for JS after load |
| `includeTags` | `string[]` | — | HTML tags to keep |
| `excludeTags` | `string[]` | — | HTML tags to remove |
| `ignoreInvalidURLs` | `bool` | `true` | Skip unparseable URLs; `false` = reject the whole request |
| `proxy` | `string` | `"auto"` | `"basic"` or `"stealth"` (residential) |
| `location.country` | `string` | — | 2-letter country code for proxy egress |
| `timeout` | `number` | — | Per-URL timeout in ms |

Invalid URLs (SSRF-blocked, unparseable) are returned in `invalidURLs` on the start response and skipped from the job unless `ignoreInvalidURLs` is `false`.

---

## Key Response Fields

**Start response** (`POST /firecrawl/v2/batch/scrape`):

```
id           — UUID for polling/cancellation
url          — ready-to-use status URL
invalidURLs  — URLs that were skipped
```

**Status response** (`GET /firecrawl/v2/batch/scrape/{id}`):

```
status        — "scraping" | "completed" | "failed"
total         — total URLs in the job
completed     — URLs finished so far
creditsUsed   — credits consumed so far
expiresAt     — RFC3339 UTC expiry of this job in server memory
next          — pagination cursor URL (null when done)
data[]        — Document objects for this page
  .markdown   — page content as Markdown
  .links      — outbound link URLs (if requested)
  .metadata
    .sourceURL      — original URL
    .statusCode     — HTTP status of the page
    .proxyUsed      — "basic" or "stealth"
    .creditsUsed    — credits for this document
    .scrapeId       — per-document UUID
```

---

## Cancelling a Job

```bash
curl -s -X DELETE "https://api.fastcrw.com/firecrawl/v2/batch/scrape/$JOB_ID" \
  -H "Authorization: Bearer $CRW_API_KEY"
```

Returns `{ "success": true, "status": "cancelled", "message": "..." }`.

---

## Checking Errors

URLs that fail mid-job are recorded but don't fail the entire batch. Retrieve them after the job completes:

```bash
curl -s "https://api.fastcrw.com/firecrawl/v2/batch/scrape/$JOB_ID/errors" \
  -H "Authorization: Bearer $CRW_API_KEY"
```

Returns `{ "success": true, "errors": [...], "robotsBlocked": [] }`.
