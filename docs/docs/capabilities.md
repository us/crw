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
  "version": "0.16.0",
  "llm": {
    "providers": ["anthropic", "openai", "deepseek", "openai-compatible", "azure"],
    "supportsBaseUrl": true,
    "serverKeyConfigured": false,
    "maxConcurrency": 0
  },
  "formats": {
    "supported": [
      "markdown", "html", "rawHtml", "plainText",
      "links", "json", "summary", "changeTracking"
    ],
    "changeTrackingModes": ["gitDiff", "json"]
  },
  "search": {
    "answer": true,
    "summarizeResults": true
  },
  "documents": {
    "parsers": ["pdf"],
    "fileUpload": {
      "supported": true,
      "endpoint": "/v2/parse",
      "maxBytes": 52428800,
      "types": ["application/pdf"],
      "ocr": false
    }
  }
}
```

All keys use camelCase.

## Fields

### `version`

`string` — The server binary's semver version (`CARGO_PKG_VERSION`).

### `llm`

| Field | Type | Description |
|-------|------|-------------|
| `providers` | `string[]` | LLM provider tags the server's dispatch layer supports. Always `["anthropic", "openai", "deepseek", "openai-compatible", "azure"]` — these are compile-time constants. |
| `supportsBaseUrl` | `boolean` | Always `true`. Any provider can be redirected to a custom base URL per request. |
| `serverKeyConfigured` | `boolean` | `true` when a server-side `[extraction.llm]` API key is configured. `false` means the instance has no server LLM key and requires callers to supply `llmApiKey` in the request body. Hosted SaaS sets this to `false` and relies on per-request keys. |
| `maxConcurrency` | `number` | Server-side fan-out cap for LLM calls. `0` when no server LLM config is present. |
| `requireByokHeader` | `string?` | Present only when the server requires a specific request header on LLM-touching calls. Absent when no header guard is configured. |

### `formats`

| Field | Type | Description |
|-------|------|-------------|
| `supported` | `string[]` | Output formats this instance can produce. The hosted API always returns all 8: `markdown`, `html`, `rawHtml`, `plainText`, `links`, `json`, `summary`, `changeTracking`. |
| `changeTrackingModes` | `string[]` | Diff modes available when `changeTracking` is in `supported`. Values: `gitDiff`, `json`. Empty on instances where `changeTracking` is unavailable. |

### `search`

| Field | Type | Description |
|-------|------|-------------|
| `answer` | `boolean` | Whether the instance supports LLM-synthesized answers on `POST /v1/search`. |
| `summarizeResults` | `boolean` | Whether per-result summaries are available on the search endpoint. |

:::note
The `search` object does not expose whether a SearXNG backend is reachable. A live search configuration check happens at request time — if SearXNG is missing, the `/v1/search` endpoint returns HTTP 503 with `error_code: "search_disabled"`.
:::

### `documents`

| Field | Type | Description |
|-------|------|-------------|
| `parsers` | `string[]` | Active document parser types. `["pdf"]` when the PDF feature is compiled in and `[document] enabled = true` in config. `[]` when PDF is disabled or not compiled. |
| `fileUpload.supported` | `boolean` | `true` when `POST /v2/parse` accepts uploads. Matches `parsers` non-empty. |
| `fileUpload.endpoint` | `string` | Always `/v2/parse` when uploads are supported. |
| `fileUpload.maxBytes` | `number` | Maximum accepted upload size in bytes (default 52 428 800 = 50 MiB). Set by `[document] max_upload_bytes` in config. |
| `fileUpload.types` | `string[]` | Accepted MIME types. `["application/pdf"]` when PDF is enabled, `[]` otherwise. |
| `fileUpload.ocr` | `boolean` | Always `false`. The built-in PDF parser does not include an OCR engine — scanned or image-only PDFs yield empty or partial text. |

## Hosted API Values

On `api.fastcrw.com` you can expect:

- `llm.serverKeyConfigured` — `false` (SaaS relies on per-request keys)
- `documents.parsers` — `["pdf"]` (PDF support is enabled)
- `documents.fileUpload.maxBytes` — `52428800` (50 MiB)
- `formats.supported` — all 8 formats
- `search.answer` and `search.summarizeResults` — `true`

## Using Capabilities at Runtime

### Agents and SDKs

Call `GET /v1/capabilities` once on startup and cache the result. Use it to:

- decide whether to send `formats: ["json"]` (check `formats.supported` contains `"json"`)
- decide whether to supply `llmApiKey` in the body (check `llm.serverKeyConfigured`)
- decide whether to offer file-upload UI or API paths (check `documents.fileUpload.supported`)
- confirm whether `changeTracking` is available before using monitor scrapes (check `formats.supported` contains `"changeTracking"`)

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

- [Output Formats](/docs/output-formats) — the 8 formats and when to use each
- [PDF Parsing](/docs/pdf-parsing) — the `POST /v2/parse` upload endpoint
- [Search](/docs/search) — `POST /v1/search` and the LLM answer path
- [Self-Hosting](/docs/self-hosting) — configuration reference for `[document]`, `[extraction.llm]`, and `[search]`
