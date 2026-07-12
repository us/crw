# Tavily ↔ CRW Compatibility Matrix

**Date verified:** 2026-05-09
**CRW version:** main (post-`/v1/search` endpoint)
**Tavily reference:** https://docs.tavily.com (verified 2026-05-09)

---

## Summary verdict

**Branch B — partial compatibility.** CRW is Tavily-*style*, not Tavily drop-in. Endpoints and concepts overlap; **request param names, response field names, error envelopes, and feature surface differ enough that a thin adapter is required**. SEO copy must use **"Tavily-style"** or **"Tavily migration"** language until/unless a Tavily-shape adapter ships in `crw-opencore`.

This decision drives Phase 1 hub title selection, Phase 4 README/Show HN copy, and the gate on `/alternatives/open-source-tavily` (per plan iter-6 Branch B).

---

## Endpoint-by-endpoint matrix

### Search

| Surface | Tavily | CRW | Compatible? |
|---|---|---|---|
| Path | `POST https://api.tavily.com/search` | `POST /v1/search` (self-host) or `POST https://fastcrw.com/api/v1/search` | adapter required |
| Auth | `Authorization: Bearer tvly-<key>` (required) | `Authorization: Bearer <key>` (optional self-host; required hosted) | similar shape |
| `query` | string, required | string, required (1–2000 chars) | ✅ |
| Result count | `max_results` (default 5, 0–20) | `limit` (default 5, max 20) | rename only |
| Time filter | `time_range` (`day`/`week`/`month`/`year` or `d`/`w`/`m`/`y`) + `start_date`/`end_date` (YYYY-MM-DD) | `tbs` (`qdr:h`, `qdr:d`, `qdr:w`, `qdr:m`, `qdr:y`) | semantic match; CRW collapses `qdr:h` to `day` |
| Topic | `topic` (`general`/`news`/`finance`) | `sources: ["web"\|"news"\|"images"]` | overlaps for general/news; **no `finance` topic in CRW**; CRW adds `images` |
| Search depth | `search_depth` (`basic`/`advanced`/`fast`/`ultra-fast`) | none | **gap — CRW has no depth control** |
| Domain filter | `include_domains` (max 300), `exclude_domains` (max 150) | none | **gap — not supported** |
| Country | `country` (195+) | none | **gap — not supported** |
| Categories | none | `categories: ["github"\|"research"\|"pdf"]` (max 5) | CRW-only feature |
| Language | none directly | `lang` (e.g., `"en"`, `"tr"`) | CRW-only |
| Answer synthesis | `include_answer` (`true`/`false`/`basic`/`advanced`) | none | **gap — no LLM answer in CRW** |
| Raw content | `include_raw_content` (`true`/`false`/`markdown`/`text`) | `scrapeOptions.formats` (`markdown`/`html`/`rawHtml`/`links`) — runs scrape pipeline per result | shape differs; CRW does full scrape |
| Images | `include_images`, `include_image_descriptions` | `sources: ["images"]` | concept differs (Tavily attaches images to results; CRW returns image bucket) |
| Date range | `start_date`/`end_date` | none | **gap — only relative `tbs`** |
| `auto_parameters`, `exact_match`, `safe_search`, `chunks_per_source`, `include_favicon`, `include_usage` | yes | none | **gaps** |

**Response shape:**

| Field (Tavily) | Field (CRW flat) | Match? |
|---|---|---|
| `query` (echoed) | not echoed | gap |
| `answer` | not present | gap (no synthesis) |
| `images` (top-level) | only via `sources: ["images"]` (grouped shape) | semantic differ |
| `results[].title` | `data[].title` | ✅ |
| `results[].url` | `data[].url` | ✅ |
| `results[].content` | `data[].description` | **rename: `content` → `description`** |
| `results[].score` (float; range unspecified in Tavily docs) | `data[].score` (float, optional, search-backend-derived) | scale differs (treat as ordinal, not absolute) |
| `results[].raw_content` | `data[].markdown` / `data[].html` / `data[].raw_html` (when `scrapeOptions` set) | CRW separates by format |
| `results[].favicon` | not present | gap |
| `results[].published_date` (news only) | `data[].published_date` (news only) | ✅ |
| top-level `response_time` | not present | gap |
| top-level `usage.credits` | not present (self-host) | n/a |
| top-level `request_id` | not present | gap |
| `data` envelope wrapper | `{success: true, data: [...]}` (or `{success, data: {web, news, images}}` grouped) | **CRW wraps; Tavily flat** |

### Extract

| Surface | Tavily | CRW | Compatible? |
|---|---|---|---|
| Path | `POST /extract` | `POST /v1/scrape` (single) | not parity — Tavily extracts batches; CRW scrapes one URL per call |
| URLs | `urls: string \| string[]` (max 20) | `url: string` (single) | **gap — no batch on CRW** |
| Depth | `extract_depth` (basic/advanced) | n/a | gap |
| Format | `format` (markdown/text) | `formats: ["markdown"\|"html"\|"rawHtml"\|"links"\|"plainText"\|"json"]` | CRW richer |
| LLM extract | none | `extract`, `json_schema` | CRW-only |
| Response | `{results, failed_results, response_time, usage, request_id}` | `{success, data: {markdown?, html?, ..., metadata}}` | divergent |

### Crawl

| Surface | Tavily | CRW | Compatible? |
|---|---|---|---|
| Path | `POST /crawl` | `POST /v1/crawl` | similar concept |
| Scope params | `instructions`, `max_depth` (1–5), `max_breadth` (1–500), `limit` (50), `select_paths`, `select_domains`, `exclude_paths`, `exclude_domains`, `allow_external` | CRW has its own: `limit`, `max_depth`, `include_paths`, `exclude_paths`, `allow_subdomains`, `webhook` | overlapping but **distinct param names** |

### Map

| Surface | Tavily | CRW | Compatible? |
|---|---|---|---|
| Path | `POST /map` | `POST /v1/map` | similar concept |
| Returns | `{base_url, results: string[]}` | `{success, data: {links: [...]}}` | shape differs |

### Research

| Surface | Tavily | CRW | Compatible? |
|---|---|---|---|
| Endpoint | `POST /research` (async, SSE streaming variant) | none | **gap — CRW has no agentic research endpoint** |

### MCP

| Surface | Tavily | CRW | Compatible? |
|---|---|---|---|
| Server | hosted SSE + local stdio (`tavily-mcp@0.1.3`) | local stdio + HTTP via `crw-mcp` | both ship MCP |
| Tools | `tavily-search`, `tavily-extract` | `crw_search`, `crw_scrape`, `crw_crawl`, `crw_map`, `crw_check_crawl_status` | CRW exposes more surface |

### Errors

| Surface | Tavily | CRW |
|---|---|---|
| Envelope | `{ "detail": { "error": "string" } }` | `{ "success": false, "error": "..." }` (CrwError variants) |
| Status codes | 400 / 401 / 403 / 429 / 432 / 433 / 500 | 400 / 401 / 408 / 429 / 500 / 503 (search_disabled) |
| Rate-limit headers | `retry-after` only | depends on hosted layer |

### Pagination

Neither side supports cursor or offset pagination on `/search`. Tavily caps at `max_results: 20`, CRW caps at `limit: 20`. Parity at the cap; no parity below.

### Streaming

Tavily streams `/research` only. CRW does not stream `/v1/search`. **No parity.**

---

## Adapter shim sketch (Branch B unblock for Phase 2 `/alternatives/open-source-tavily`)

A minimal Python shim that lets a Tavily client point at CRW. Drop into the user's code, swap `client = TavilyClient(api_key=...)` for the wrapper:

```python
import requests

class CrwTavilyShim:
    """Adapt a Tavily-style call to CRW /v1/search.

    Caveats vs real Tavily:
      - No `answer` synthesis (returns "" if include_answer is set).
      - No `include_domains` / `exclude_domains` / `country` filtering.
      - `topic="finance"` is not supported.
      - `score` is search-backend-derived; treat as ordinal not absolute.
      - `raw_content` requires scrapeOptions on CRW; this shim wires it through.
    """
    def __init__(self, base_url: str, api_key: str | None = None):
        self.base_url = base_url.rstrip("/")
        self.headers = {"Content-Type": "application/json"}
        if api_key:
            self.headers["Authorization"] = f"Bearer {api_key}"

    def search(
        self,
        query: str,
        *,
        max_results: int = 5,
        topic: str = "general",
        time_range: str | None = None,
        include_raw_content: bool | str = False,
        **_unsupported,
    ) -> dict:
        sources = {"general": ["web"], "news": ["news"]}.get(topic, ["web"])
        body = {"query": query, "limit": max_results, "sources": sources}
        if time_range:
            tbs_map = {"day": "qdr:d", "week": "qdr:w", "month": "qdr:m", "year": "qdr:y"}
            body["tbs"] = tbs_map.get(time_range, time_range)
        if include_raw_content:
            body["scrapeOptions"] = {"formats": ["markdown"], "onlyMainContent": True}
        r = requests.post(f"{self.base_url}/v1/search", json=body, headers=self.headers)
        r.raise_for_status()
        data = r.json()["data"]
        web = data["web"] if isinstance(data, dict) and "web" in data else data
        return {
            "query": query,
            "answer": "",  # CRW does not synthesize answers
            "results": [
                {
                    "title": item["title"],
                    "url": item["url"],
                    "content": item.get("description", ""),
                    "score": item.get("score"),
                    "raw_content": item.get("markdown"),
                }
                for item in web
            ],
        }
```

This shim is the artifact `/alternatives/open-source-tavily` should ship to honor Branch B's "delay until Tavily-client adapter ships" condition. Without it, Phase 2 `/alternatives/open-source-tavily` should narrow to "Open-source, self-hosted search APIs" per plan iter-6.

---

## Decisions falling out of this matrix

1. **Branch B applies.** Hub title cannot use "Tavily-compatible" without qualification. Two approved title variants (per plan iter-7):
   - `"Tavily-Style Search API — Free to Self-Host (2026)"` (53 chars)
   - `"Migrate from Tavily — Self-Hosted Search API (2026)"` (52 chars)
2. **README repo description (Phase 4 step 1):** `"Free, self-hostable search API for AI agents — built in Rust. Tavily-style endpoints."` (Branch B variant).
3. **`/alternatives/open-source-tavily` ships with the adapter shim above** referenced inline (no delay), since the shim removes the "Tavily-client adapter must ship first" blocker.
4. **The plan's "drop-in replacement" copy is forbidden** until either (a) param names align, or (b) the adapter ships and is tested in CI. Use "drop-in via adapter shim" only.
5. **Compatibility gate for the `tavily compatible api` keyword** (per Phase 0 priority table): redirect to `/alternatives/tavily#migration` anchor that documents the matrix above. Do not promise drop-in.

---

## What this matrix does NOT cover (out of scope)

- Latency benchmark — separate Phase 0 deliverable in `crw-opencore/bench/tavily-rerun-2026-05-09/`.
- Quality eval — separate Phase 0 deliverable in `crw-opencore/bench/quality-eval-2026-05-09/`.
- Pricing math — separate Phase 0 deliverable in `crw-saas/seo-baselines/pricing-math.md`.

This file is the **single source of truth for endpoint/request/response/auth/error/pagination/streaming compatibility**. Update it when either CRW adds endpoints or Tavily ships breaking changes; bump the "verified" date in the header.
