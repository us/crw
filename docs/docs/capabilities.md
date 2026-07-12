# Capabilities

`GET /v1/capabilities` returns a JSON snapshot of what the running instance supports. Clients and agents can call it on startup to adapt their behavior — instead of guessing what a self-hosted deployment has configured, ask.

## Request

```http
GET https://api.fastcrw.com/v1/capabilities
Authorization: Bearer YOUR_API_KEY
```

No request body. No query parameters.

## Response Shape

```json
{
  "version": "0.23.0",
  "llm": {
    "providers": ["anthropic", "openai", "deepseek", "openai-compatible", "azure"],
    "supportsBaseUrl": true,
    "serverKeyConfigured": false,
    "maxConcurrency": 0
  },
  "formats": {
    "supported": [
      "markdown", "html", "rawHtml", "plainText",
      "links", "json", "summary", "changeTracking", "screenshot"
    ],
    "llmRequired": ["json", "summary"],
    "changeTrackingModes": ["gitDiff", "json"],
    "changeTrackingModesLlmRequired": ["json"]
  },
  "search": {
    "supported": true,
    "answer": false,
    "summarizeResults": false
  },
  "screenshot": {
    "supported": true,
    "fullPage": true
  },
  "renderers": {
    "available": ["lightpanda", "chrome"],
    "mode": "auto"
  },
  "extract": {
    "supported": true,
    "maxUrls": 50,
    "perFieldAttribution": false
  },
  "documents": {
    "parsers": ["pdf"],
    "fileUpload": {
      "supported": true,
      "endpoint": "/firecrawl/v2/parse",
      "maxBytes": 52428800,
      "types": ["application/pdf"],
      "ocr": false
    }
  },
  "limits": {
    "maxBatchUrls": 10000,
    "maxExtractUrls": 50,
    "searchDefaultLimit": 5,
    "searchMaxLimit": 20,
    "maxUploadBytes": 52428800
  }
}
```

All keys use camelCase.

## The Contract

Every boolean in this response is **derived from the real build (compiled cargo features) and the effective config**. Nothing is hardcoded to `true`.

A capability reads `true` only when the instance can perform the operation **for a well-formed request that supplies no extra credentials**. Credentials are reported separately rather than folded into the booleans:

- `llm.serverKeyConfigured` tells you whether a server-side LLM key exists.
- `formats.llmRequired` tells you which formats need an LLM at all.

Bring-your-own-key is always accepted and cannot be turned off. So on a BYOK-only deployment you will see `search.supported: true` alongside `search.answer: false`: the instance cannot answer on its own, but a request that carries `llmApiKey` still gets an answer. **Gate on `search.supported` if you supply your own key; gate on `search.answer` if you do not.**

## Fields

### `version`

`string` — The server binary's semver version (`CARGO_PKG_VERSION`).

### `llm`

| Field | Type | Description |
|-------|------|-------------|
| `providers` | `string[]` | LLM provider tags the dispatcher accepts. Sourced from the same constant the dispatcher validates against, so the advertised list cannot drift from the accepted one. |
| `supportsBaseUrl` | `boolean` | The dispatcher accepts a custom `baseUrl` on the scrape and search paths. Note: `POST /v1/extract` rejects `baseUrl` — for that route the endpoint is configured server-side via `[extraction.llm] base_url`. |
| `serverKeyConfigured` | `boolean` | `true` when a server-side `[extraction.llm]` API key is configured. `false` means the instance has no server LLM key and requires callers to supply `llmApiKey` in the request body. Hosted SaaS sets this to `false` and relies on per-request keys. |
| `maxConcurrency` | `number` | Server-side fan-out cap for LLM calls. `0` when no server LLM config is present. |
| `requireByokHeader` | `string?` | Present only when the server requires a specific request header on LLM-touching calls. Absent when no header guard is configured. |

### `formats`

| Field | Type | Description |
|-------|------|-------------|
| `supported` | `string[]` | Output formats this build and config can actually produce. Eight are always present: `markdown`, `html`, `rawHtml`, `plainText`, `links`, `json`, `summary`, `changeTracking`. A ninth, `screenshot`, appears only when a capture-capable renderer is compiled in and configured — it is listed here exactly when `screenshot.supported` is `true`. |
| `llmRequired` | `string[]` | Formats that additionally need an LLM: a server key (`llm.serverKeyConfigured`) or a per-request `llmApiKey`. Requesting one without a key is a hard error, never a silent downgrade. |
| `changeTrackingModes` | `string[]` | Diff modes available when `changeTracking` is in `supported`. Values: `gitDiff`, `json`. |
| `changeTrackingModesLlmRequired` | `string[]` | Change-tracking modes that need an LLM. `gitDiff` is deterministic and needs none; `json` mode calls an LLM. |

### `search`

| Field | Type | Description |
|-------|------|-------------|
| `supported` | `boolean` | `POST /v1/search` is usable: `[search] enabled` is on and a backend URL is configured. |
| `answer` | `boolean` | LLM-synthesized answers work **without a caller-supplied key** — that is, `supported` is `true` **and** a server LLM key is configured. When this is `false` but `supported` is `true`, a request carrying `llmApiKey` still gets an answer. |
| `summarizeResults` | `boolean` | Per-result summaries. Same gate as `answer`. |

:::note
`search.supported` reflects configuration, not a live health probe. A configured-but-unreachable backend still reports `true`; at request time `/v1/search` returns HTTP 503 with `error_code: "search_disabled"` when search is off, and a `422` when the backend is unreachable.
:::

### `screenshot`

| Field | Type | Description |
|-------|------|-------------|
| `supported` | `boolean` | A capture-capable renderer is compiled in **and** configured. Capture runs over CDP `Page.captureScreenshot`, so it needs `chrome`, `chrome_proxy` or `playwright`. LightPanda and Camoufox cannot capture: an instance holding only those reports `false`, and the scrape path fails closed on a screenshot request rather than returning an empty image. |
| `fullPage` | `boolean` | Full-page capture (`screenshot@fullPage` on v2, `screenshotFullPage` on v1). Same gate as `supported`. |

### `renderers`

| Field | Type | Description |
|-------|------|-------------|
| `available` | `string[]` | The JS renderer tiers this instance actually constructed, in fallback order. Reflects both the build (a binary without the `cdp` feature constructs none) and the config (a tier with no endpoint set is never built). These are exactly the values the per-request `renderer` pin accepts. |
| `mode` | `string` | Effective `[renderer] mode` — e.g. `auto`, `none`, or a pinned tier. |
| `renderJsDefault` | `boolean?` | Effective `[renderer] render_js_default`. Omitted when unset (auto-detect). |

:::note
`available` means "constructible and pinnable", not "always in the auto ladder". `camoufox` is built whenever its endpoint is set even when it is excluded from auto, and `chrome_proxy` is held out of the auto ladder as a hard-block recovery arm when `auto_egress_escalation` is on.
:::

### `extract`

| Field | Type | Description |
|-------|------|-------------|
| `supported` | `boolean` | `POST /v1/extract` (async, multi-URL) is mounted. Each URL still needs an LLM — see `llm.serverKeyConfigured` and BYOK. |
| `maxUrls` | `number` | Max URLs accepted per request (`[crawler] max_extract_urls`). |
| `perFieldAttribution` | `boolean` | Per-field `basis` attribution. `false` today: `/v1/extract` rejects `basis: true` outright rather than silently ignoring it. |

### `documents`

| Field | Type | Description |
|-------|------|-------------|
| `parsers` | `string[]` | Active document parser types. `["pdf"]` when the PDF feature is compiled in and `[document] enabled = true` in config. `[]` when PDF is disabled or not compiled. |
| `fileUpload.supported` | `boolean` | `true` when the parse endpoint accepts uploads: a parser is compiled and enabled, and the enforced cap is above zero. |
| `fileUpload.endpoint` | `string` | The canonical upload path, `/firecrawl/v2/parse`. The deprecated root alias `/v2/parse` is still mounted and behaves identically. |
| `fileUpload.maxBytes` | `number` | The **enforced** upload cap in bytes — the same value the body-limit layer applies, i.e. `[document] max_upload_bytes` clamped by a 50 MiB in-memory ceiling. Lowering the knob lowers the enforced cap; raising it past the ceiling does not. |
| `fileUpload.types` | `string[]` | Accepted MIME types. `["application/pdf"]` when PDF is enabled, `[]` otherwise. |
| `fileUpload.ocr` | `boolean` | Always `false`. The built-in PDF parser does not include an OCR engine — scanned or image-only PDFs yield empty or partial text. |

### `limits`

Only enforced caps appear here. Each value is the one the server actually applies, not a documented default.

| Field | Type | Description |
|-------|------|-------------|
| `maxBatchUrls` | `number` | Max URLs per batch-scrape submission (`[crawler] max_batch_urls`). A larger batch is rejected up front. |
| `maxExtractUrls` | `number` | Max URLs per `/v1/extract` request (`[crawler] max_extract_urls`). Mirrors `extract.maxUrls`. |
| `searchDefaultLimit` | `number` | The `limit` `/v1/search` uses when the request omits it. |
| `searchMaxLimit` | `number` | Hard cap on the `/v1/search` `limit`. |
| `maxUploadBytes` | `number` | Enforced upload cap on the parse endpoint. Mirrors `documents.fileUpload.maxBytes`. |

## Reading a Deployment

The two deployment shapes differ in exactly one interesting way — where the LLM key lives.

**BYOK-fronted (a SaaS or gateway in front of the engine).** The gateway injects an LLM key on each request, so the engine itself has none: `llm.serverKeyConfigured` is `false`, and therefore `search.answer` and `search.summarizeResults` are `false` too. `search.supported` is still `true`. This is not a missing feature — it is the honest statement that the *engine alone*, with no caller-supplied key, cannot answer. Callers that send `llmApiKey` get answers.

**Self-hosted with a server key.** `[extraction.llm] api_key` is set, so `llm.serverKeyConfigured` is `true` and `search.answer` follows `search.supported`.

Everything else (renderer tiers, screenshot capture, parsers, limits) varies per deployment and cannot be predicted without asking.

## Using Capabilities at Runtime

### Agents and SDKs

Call `GET /v1/capabilities` once on startup and cache the result. Use it to:

- decide whether to send `formats: ["json"]` (check `formats.supported` contains `"json"`)
- decide whether to supply `llmApiKey` in the body (check `llm.serverKeyConfigured`, and `formats.llmRequired` for which formats need one)
- decide whether to offer screenshots (check `screenshot.supported` — never assume it from the presence of a renderer)
- decide whether to offer file-upload UI or API paths (check `documents.fileUpload.supported`, and size the client-side guard from `limits.maxUploadBytes`)
- confirm whether `changeTracking` is available before using monitor scrapes (check `formats.supported` contains `"changeTracking"`)
- size batch requests against `limits.maxBatchUrls` instead of discovering the cap through a rejection

```python
import httpx

caps = httpx.get(
    "https://api.fastcrw.com/v1/capabilities",
    headers={"Authorization": "Bearer YOUR_API_KEY"},
).json()

if caps["llm"]["serverKeyConfigured"]:
    llm_key_in_body = None          # server has a key
else:
    llm_key_in_body = "sk-..."      # caller must supply a per-request key

pdf_ok = caps["documents"]["fileUpload"]["supported"]
can_screenshot = caps["screenshot"]["supported"]

# Gate on `supported` when you bring your own key; on `answer` when you do not.
if llm_key_in_body:
    can_answer = caps["search"]["supported"]
else:
    can_answer = caps["search"]["answer"]
```

### Self-Hosted Instances

Self-hosted deployments differ from the hosted API in several ways a caller cannot predict without checking:

- PDF parsing may be disabled (`[document] enabled = false`) or not compiled in
- A server-side LLM key may or may not be configured
- The `[document] max_upload_bytes` limit can be set below 50 MiB
- SearXNG may not be connected

Always call `/v1/capabilities` before making assumptions about what a self-hosted instance can do.

## Authentication

`GET /v1/capabilities` is an authenticated route on both hosted and self-hosted deployments (when API keys are configured). The response contains no secrets, but it describes deployment configuration, which is worth protecting.

On a self-hosted instance with no API keys configured, the route is public.

## What To Read Next

- [Output Formats](/docs/output-formats) — the 9 formats and when to use each
- [PDF Parsing](/docs/pdf-parsing) — the `POST /firecrawl/v2/parse` upload endpoint
- [Search](/docs/search) — `POST /v1/search` and the LLM answer path
- [Self-Hosting](/docs/self-hosting) — configuration reference for `[document]`, `[extraction.llm]`, and `[search]`
