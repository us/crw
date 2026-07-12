# Firecrawl ↔ fastCRW Capability Matrix

**Date:** 2026-05-11
**Owner:** content/growth (autonomous run)
**Re-verify within 48h of any page citing this doc.**
**Sources:** `docs.firecrawl.dev/sdks/rust` and `github.com/firecrawl/firecrawl/blob/main/SELF_HOST.md` (Firecrawl repo on `main` as of 2026-05-11 — claims dependent on the exact Firecrawl self-host stack are stated as "as of 2026-05-11" and must be re-verified before any page citing this matrix ships); `firecrawl.dev/pricing`; fastCRW `crates/crw-server/src/routes/`, `crates/crw-search/src/`, `Cargo.toml`.

This is a **capability matrix**, not an API-shape compatibility matrix (which the parent plan's `COMPATIBILITY.md` already provides for Tavily). Capabilities here = what each product can actually do, not just whether endpoint names match.

> **Honest framing:** Firecrawl is a feature-richer cloud-first product with an OSS self-host story that has documented gaps. fastCRW is a single-binary Rust-native alternative with narrower scope but better self-host ergonomics and a Tavily-style search API on top. These products overlap on `/scrape`, `/crawl`, `/map`, `/search`; they diverge on LLM extraction, deep research, anti-bot depth, and deployment surface. We do NOT claim drop-in equivalence — claim is "Rust-native alternative for the overlap surface."

> **Supported compat target — Firecrawl v2 only (as of 2026-07-04):** Firecrawl drop-in compatibility is served under the dedicated **`/firecrawl/*`** namespace and targets **Firecrawl's v2 API/SDK**. `/firecrawl/v2/*` is verified end-to-end for `scrape`, `search`, `extract`, `map`, `crawl`, and `batch`. **How a Firecrawl SDK reaches it depends on the language:** the **JS/TS SDK** (`@mendable/firecrawl-js`) concatenates `apiUrl` + path, so `apiUrl: "…/firecrawl"` lands on `/firecrawl/v2/*` (verified via server access logs); the **Python SDK** (`firecrawl-py`) resolves endpoint paths against the host root via `urljoin`, dropping a `/firecrawl` suffix — so it lands on the equivalent root `/v2/*` (the same handlers). Point Python clients at the bare host. The **legacy v1 Firecrawl SDK is not a supported drop-in**: several v1 responses (`search`, `map`, crawl-status, `extract`) use fastCRW's own response shapes and don't deserialize under the strict v1 SDK models — `scrape` is the only fully v1-SDK-compatible route. fastCRW's own native API remains `/v1/*` at the root (what the public docs center on); root `/v2/*` is a deprecated alias of `/firecrawl/v2/*`, kept for backward-compat.

---

## 1. Endpoint coverage (high-level)

| Capability | Firecrawl Cloud | Firecrawl self-host (OSS) | fastCRW |
|---|---|---|---|
| `/v1/scrape` (single URL → markdown/html) | ✅ | ✅ (no Fire-engine) | ✅ |
| `/v1/crawl` (multi-page) | ✅ | ✅ | ✅ |
| `/v1/map` (URL discovery) | ✅ | ✅ | ✅ |
| `/v1/search` (web search → grounded results) | ✅ | ⚠️ (no Fire-engine; Cloud has stronger anti-bot) | ✅ (own search backend) |
| `/v1/extract` (LLM extraction) | ✅ (standalone route) | ⚠️ (requires LLM provider key + manual `.env`) | ✅ **Standalone `POST /v1/extract`** (async: returns a job id, poll `GET /v1/extract/{id}`). Accepts `urls: [...]` (multi-URL, capped by `limits.maxExtractUrls`). `/v1/scrape` with `formats: ["json"]` + `jsonSchema` also works for a single URL. Needs an LLM key (server-side or per-request). |
| `/v1/deep-research` | ✅ | ❌ (Cloud-only) | ❌ |
| `/firecrawl/v2/parse` (file upload → markdown) | ✅ (PDF/DOCX/XLSX/HTML/…) | ⚠️ (rolling out) | ✅ **PDF only** (multipart `file` + `options`; pure-Rust `pdf-inspector`, no OCR) |
| `/v1/agent` (Spark models) | ✅ | ❌ | ❌ |

**Source:** `github.com/firecrawl/firecrawl/blob/main/SELF_HOST.md`. Capture commit hash and pin per page citing this row before publish.

### Document parsing (`parsers` + `/firecrawl/v2/parse`)

- **`parsers` on `/firecrawl/v2/scrape`** — Firecrawl-compatible. A URL returning
  `application/pdf` is auto-converted to markdown by default (no field needed,
  matches Firecrawl). Accepts both `parsers: ["pdf"]` and
  `parsers: [{ "type": "pdf", "mode": "auto"|"fast"|"ocr", "maxPages": N }]`.
  `parsers: []` disables conversion (raw bytes).
- **`mode`** — accepted for wire-compatibility. fastCRW has **no OCR**, so
  `mode: "ocr"` (and the OCR-fallback half of `auto`) degrades to text-layer
  extraction with a `pdf_ocr_unsupported` / `pdf_scanned` warning rather than an
  error. `fast` (text-only) is the native behavior.
- **`POST /firecrawl/v2/parse`** — multipart `file` + `options` (JSON string), same
  response envelope as `/firecrawl/v2/scrape`. fastCRW supports **PDF only** (the
  `pdf-inspector` engine); Firecrawl additionally parses DOCX/XLSX/RTF/ODT.
  Advertised at `GET /firecrawl/v2/capabilities` → `documents.parsers` / `fileUpload` so
  callers can detect support before sending.

---

## 2. Authentication

| | Firecrawl | fastCRW |
|---|---|---|
| Header | `Authorization: Bearer <key>` | `Authorization: Bearer <key>` (configurable; default `X-API-Key` for self-host) |
| Self-host auth bypass | Optional via env var | Optional via `[server.auth_required = false]` in `config.toml` |
| Per-key rate limits | Yes (Cloud) | Yes (per-key tier — see `crates/crw-server/src/routes/`) |

**Surface match:** Bearer style — yes. Param/header naming — divergent if user customizes.

---

## 3. Request/response shape (overlap surface — `/v1/scrape`, `/v1/crawl`, `/v1/map`, `/v1/search`)

> **Reference for full diff:** parent plan's `COMPATIBILITY.md` covers Tavily-shape comparison. For Firecrawl shape comparison: pin in next iteration when `crw-server/tests/` has the cross-vendor compat fixtures.

| Field | Firecrawl | fastCRW |
|---|---|---|
| Request `url` | string | string ✅ |
| Request `formats` | `["markdown", "html", ...]` | `["markdown", "html", …]` ✅ (`extract` / `llm-extract` are accepted aliases for `json`) |
| Request `onlyMainContent` | boolean | boolean ✅ |
| Request `waitFor` (ms) | number | number ✅ |
| Response `data.markdown` | string | string ✅ |
| Response `data.metadata` | object (title, description, language, sourceURL...) | object (similar) — **field-name divergence on a few keys; needs row-level diff** |
| Response `success` | boolean | boolean ✅ |
| Crawl `data.completed` polling | required | required ✅ |
| Error envelope | `{ success: false, error: "..." }` | similar; **divergence on error code naming** — needs row-level diff |

**Action item for next iteration of this doc:** add concrete field-by-field diff for `metadata` and error envelope before any page claims "drop-in" or "API-compatible." Today the page copy says "Rust-native alternative for the overlap surface" — defensible without exhaustive shape match.

---

## 4. Rust SDK status

| | Firecrawl | fastCRW |
|---|---|---|
| Official Rust SDK | ✅ `firecrawl` crate on crates.io | N/A (no SDK; HTTP API only — `reqwest` example in docs) |
| Self-host constructor | `Client::new_selfhosted(api_url, api_key)` per docs.firecrawl.dev/sdks/rust (verify in v1 — v2 split documented; constructor naming differs across doc versions) | N/A |
| Crate version pin (lock before any page cites) | **TODO** — capture exact crate version + verify constructor in that version + commit a CI demo before T8 spoke ships Path 1 copy. If unverifiable, T8 ships Path-2-only. | N/A |

**Plan iter-3 critical:** the `Client::new_selfhosted` signature is documented but the v1/firecrawl/v2 split means the Rust SDK lags v2 features. Before the T8 spoke claims "official Rust SDK works against self-hosted Firecrawl," a CI demo must succeed against the locked crate version. If CI infra isn't stood up (Firecrawl Docker stack: Postgres + Redis + workers), T8 collapses Path 1 to "official Rust SDK exists; CI demo TODO" pointer and ships Path 2 (fastCRW Rust-native alternative) as primary intent.

---

## 5. LLM extraction (`/extract`)

| | Firecrawl Cloud | Firecrawl self-host | fastCRW |
|---|---|---|---|
| Schema-based extraction (single URL) | ✅ via `/v1/extract` | ⚠️ (requires manual LLM key in `.env`) | ✅ via `POST /v1/extract`, or via `/v1/scrape` with `formats: ["json"]` + top-level `jsonSchema` (Firecrawl-compatible alias: `extract.schema`; LLM key configured in `[extraction.llm]` or passed per request) |
| Multi-URL `/extract` (one call → many URLs) | ✅ | ⚠️ | ✅ via `POST /v1/extract` with `urls: [...]` (capped by `limits.maxExtractUrls`, default 50) |
| Provider support | OpenAI, Anthropic, etc. | Same (manual config) | OpenAI, Anthropic, configurable via `[extraction.llm]` |
| Pricing | Per call + LLM token cost | Self-paid LLM tokens only | Self-paid LLM tokens only (self-host); cloud per-call pricing on managed plans |

**Shape note:** fastCRW's `/v1/extract` is async — it returns a job id you poll on `GET /v1/extract/{id}`, and it accepts a `urls` array. The per-request URL cap is advertised as `limits.maxExtractUrls` on `GET /v1/capabilities` (default 50). Single-URL extraction also works inline via `/v1/scrape` `formats: ["json"]` with a top-level `jsonSchema` (or the `extract.schema` alias for closer Firecrawl parity). Per-field `basis` attribution is supported: pass `basis: true` to get, for each top-level scalar field, an evidence object with its source URL, a hash of the exact text the model saw, a verbatim excerpt, a confidence, and an honest status (a field that cannot be grounded is marked `unverified`/`unsupported`, never fabricated). `perFieldAttribution` on `GET /v1/capabilities` reports whether this build ships it.

---

## 6. Deep research (`/v1/deep-research`)

| | Firecrawl Cloud | Firecrawl self-host | fastCRW |
|---|---|---|---|
| Multi-step web research | ✅ (Spark 1 backend) | ❌ (Cloud-only feature) | ❌ |

**Honest divergence:** Cloud-only Firecrawl feature. fastCRW does not match.

---

## 7. MCP server

| | Firecrawl | fastCRW |
|---|---|---|
| MCP server bundled | ✅ (`firecrawl-mcp-server`) | ✅ (built-in `crw-mcp` crate; `crw_scrape`, `crw_crawl`, `crw_check_crawl_status`, `crw_map`, `crw_search`, `crw_parse_file` tools) |

**Surface match:** both products ship MCP. Tool names differ (Firecrawl uses `firecrawl_*`, fastCRW uses `crw_*`); semantic mapping is straightforward.

**Structured output:** `crw_search` additionally emits MCP-2025-06-18 `structuredContent` shaped to fastCRW's own `/v1/search` envelope (`data.results`), **not** Firecrawl's `data.web` shape — it mirrors the body fastCRW clients already consume, so this is not a Firecrawl-shape parity claim. The legacy text content block is retained for lenient clients.

---

## 8. Anti-bot / Fire-engine

| | Firecrawl Cloud | Firecrawl self-host | fastCRW |
|---|---|---|---|
| Fire-engine (Cloud-only proprietary) | ✅ | ❌ | ❌ |
| Browser fallback | ✅ (Playwright/Puppeteer) | ✅ (manual config) | ⚠️ (reqwest baseline; browser fallback path documented in `crates/crw-search/`) |
| Rotating IP / proxy | ✅ Cloud | ❌ (BYO proxy) | ❌ (BYO proxy) |

**Honest divergence:** Firecrawl Cloud has the best anti-bot story; self-host loses this; fastCRW is on par with Firecrawl self-host on this dimension. For high-anti-bot scenarios (cloudflare-protected, JS-heavy SPAs without API), Firecrawl Cloud is still the strongest choice.

---

## 9. Deployment surface

| | Firecrawl self-host | fastCRW |
|---|---|---|
| Stack | Docker Compose: API + workers + Postgres + Redis | Single Rust binary (or `docker compose up` with bundled search backend sidecar) |
| Memory baseline | ~1-2GB (full stack) | ~6.6 MB idle (binary); +search backend container if used |
| Cold start | ~5-15s (full stack warmup) | ~85ms (binary) |
| Languages | TypeScript (workers), some Rust (`/parse` Apr 2026) | Rust |

**fastCRW wedge:** dramatically simpler deployment surface. Single binary vs multi-service stack. This is the primary "Rust-native, self-host friendly" claim for the T8 spoke.

---

## 10. License

| | Firecrawl | fastCRW |
|---|---|---|
| License | AGPL-3.0 | AGPL-3.0 |
| Self-host commercial use | Yes, with AGPL §13 obligations | Yes, same |

**Same license.** Counsel-reviewed §13 explainer is deferred per parent plan; pages cite neutral notice + link to license.

---

## 11. Pricing (managed cloud, for reference — re-verify within 48h)

| Plan | Firecrawl Cloud | fastCRW Cloud |
|---|---|---|
| Free | 1k credits/mo | (current pricing — see `/pricing`) |
| Hobby | $16 / 5k credits | — |
| Standard | $83 / 100k | $69/mo (per `src/lib/plans-client.ts:67`) |
| Growth | $333 / 500k | — |
| Scale | $599 / 1M | — |

**Source:** `firecrawl.dev/pricing` (verify within 48h before any page ships these numbers).

---

## 12. Recent moves (Mar–May 2026)

- **Firecrawl:** Lockdown Mode (Apr 30), Rust-based `/parse` engine (Apr 28), Spark 1 models on `/agent`, multiple status incidents Mar 21/24/31 + Apr 19-30 (`status.firecrawl.dev/incidents`), Series A $14.5M (Aug 2025).
- **fastCRW:** `/v1/search` on our own search backend (Q2 2026), bundled Docker sidecar, Tavily-cluster pages shipped 2026-05-09.

---

## What this matrix authorizes

- T8 spoke and T9 deep refresh may cite this doc by URL.
- Any "compatibility" or "drop-in" copy must reference a specific row that says ✅ in both columns.
- Divergence rows must be acknowledged in page copy with the same word ("not implemented," "not supported," etc.).
- Re-verify-within-48h rule applies to: pricing, recent moves, SDK constructor, SELF_HOST.md commit hash, and Apify announcement (separate doc).

## What this matrix does NOT authorize

- Latency claims (need own benchmark; cite parent plan Tavily benchmark or wait for Firecrawl-specific benchmark).
- Anti-bot success-rate claims (no benchmark).
- LLM extraction quality claims (the feature ships, but no benchmark backs a quality comparison).
- Drop-in API equivalence claims (request/response shape diff is incomplete in §3).
