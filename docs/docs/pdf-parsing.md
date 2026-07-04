<div class="page-intro">
  <div class="page-kicker">More API</div>
  <h1>PDF Parsing</h1>
  <p class="page-subtitle">Upload a PDF and get clean markdown, plain text, or structured JSON back. One multipart request, no third-party OCR pipeline needed.</p>
  <div class="page-capabilities">
    <div class="page-capability"><strong>Best for:</strong> research papers, reports, contracts</div>
    <div class="page-capability"><strong>Route:</strong> <code>POST /firecrawl/v2/parse</code></div>
    <div class="page-capability"><strong>Max upload:</strong> 50 MiB</div>
  </div>
  <div class="page-actions">
    <a class="page-btn primary" href="#quick-start">Quick Start</a>
    <a class="page-btn secondary" href="#crawl-parse-workflow">Crawl → Parse Workflow</a>
  </div>
</div>

## PDF Parsing with CRW

### /firecrawl/v2/parse

```http
POST /firecrawl/v2/parse
Content-Type: multipart/form-data
```

Authentication:

- Hosted: send `Authorization: Bearer YOUR_API_KEY`
- Self-hosted: only required when `auth.api_keys` is configured

The route accepts a `multipart/form-data` body with two parts:

| Part | Required | Description |
|---|---|---|
| `file` | yes | The PDF file bytes. Must begin with a `%PDF-` header. |
| `options` | no | A JSON string with output options (see [Options](#options) below). |

The file must be a valid PDF. Binary files with other content types are rejected with `400 Bad Request`. A corrupt or encrypted PDF returns `422 Unprocessable Entity`. The maximum upload size is **50 MiB** (52,428,800 bytes); requests above this limit receive a `413 Content Too Large` before the body is fully read.

### Quick start {#quick-start}

:::tabs
::tab{title="cURL"}
```bash
curl -X POST https://api.fastcrw.com/firecrawl/v2/parse \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -F "file=@/path/to/document.pdf;type=application/pdf"
```
::tab{title="Python (SDK)"}
```python
from crw import CrwClient

client = CrwClient()  # reads CRW_API_KEY from env
result = client.parse_file(path="/path/to/document.pdf")
print(result["markdown"])
```
::tab{title="TypeScript (SDK)"}
```typescript
import { CrwClient } from "crw-sdk";
import { readFileSync } from "fs";

const client = new CrwClient();
const bytes = new Uint8Array(readFileSync("/path/to/document.pdf"));
const result = await client.parseFile(bytes, { filename: "document.pdf" });
console.log(result.markdown);
```
::tab{title="Python (requests)"}
```python
import requests

with open("/path/to/document.pdf", "rb") as f:
    resp = requests.post(
        "https://api.fastcrw.com/firecrawl/v2/parse",
        headers={"Authorization": "Bearer YOUR_API_KEY"},
        files={"file": ("document.pdf", f, "application/pdf")},
    )

print(resp.json()["data"]["markdown"])
```
:::

### Response

```json
{
  "success": true,
  "data": {
    "markdown": "# Annual Report 2025\n\nRevenue grew 18%...",
    "metadata": {
      "sourceURL": "upload://document.pdf",
      "statusCode": 200,
      "numPages": 12,
      "sourceFilename": "document.pdf",
      "proxyUsed": "basic",
      "cacheState": "miss",
      "concurrencyLimited": false,
      "creditsUsed": 1,
      "scrapeId": "a1b2c3d4-..."
    }
  }
}
```

Key metadata fields for PDF responses:

| Field | Description |
|---|---|
| `numPages` | Total number of pages in the document |
| `sourceFilename` | Original filename passed in the upload |
| `sourceURL` | Always `upload://<filename>` for parse requests |

## Options

Pass options as a JSON string in the `options` multipart field. All fields are optional; defaults match `/firecrawl/v2/scrape`.

| Field | Type | Default | Description |
|---|---|---|---|
| `formats` | string[] | `["markdown"]` | Output formats. See [Formats](#formats). |
| `parsers` | ParserSpec[] | `[{"type":"pdf"}]` | Parser directives. See [parsers[]](#parsers). |
| `jsonSchema` | object | — | JSON Schema for LLM extraction. Requires `formats: ["json"]`. |
| `summaryPrompt` | string | — | Custom prompt for `formats: ["summary"]`. |
| `maxContentChars` | number | — | Truncate each content field to this many characters. |

### Example with options

```bash
curl -X POST https://api.fastcrw.com/firecrawl/v2/parse \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -F "file=@/path/to/document.pdf;type=application/pdf" \
  -F 'options={"formats":["markdown","plainText"],"parsers":[{"type":"pdf","maxPages":5}]}'
```

### Formats {#formats}

`/firecrawl/v2/parse` supports a subset of the scrape formats. Formats that require a renderer (`html`, `rawHtml`, `changeTracking`) are not applicable to uploaded documents and return a warning if requested.

| Format | Description |
|---|---|
| `markdown` | Extracted text as Markdown (default) |
| `plainText` | Extracted text without Markdown syntax |
| `links` | Array of URLs found in the document |
| `json` | Structured extraction via LLM + `jsonSchema` |
| `summary` | LLM-generated prose summary |

`json` and `summary` require an LLM provider configured on the engine (or a per-request `llmApiKey`). Without one, the server returns a `400` with a clear message.

### parsers[] {#parsers}

The `parsers` field controls how the document is processed. Currently only `"pdf"` is supported.

**Accepted forms** (the server accepts both):

```json
// Short form (bare string):
"parsers": ["pdf"]

// Object form (full control):
"parsers": [{ "type": "pdf", "mode": "auto", "maxPages": 10 }]
```

| Field | Type | Default | Description |
|---|---|---|---|
| `type` | string | — | Parser type. Only `"pdf"` is supported today. |
| `mode` | string | `"auto"` | `"auto"` or `"fast"` use text extraction. `"ocr"` is accepted for wire-compatibility but fastCRW has no OCR pipeline — scanned pages return empty text with a warning. |
| `maxPages` | number | no limit | Cap the number of pages converted. Useful for very large documents where you only need the first N pages. |

:::note
fastCRW performs **text-layer extraction only**. Image-only (scanned) PDFs that have no embedded text layer will return empty or near-empty markdown. No warning is guaranteed for individual scanned pages — check `numPages` vs actual content length if you expect text.
:::

## SDK usage

### Python SDK — `client.parse_file()`

The Python SDK `parse_file()` works in both HTTP mode (cloud or self-hosted server) and local subprocess mode.

```python
from crw import CrwClient

client = CrwClient()  # CRW_API_KEY from env

# From a file on disk:
result = client.parse_file(path="/path/to/report.pdf")

# From bytes already in memory:
with open("/path/to/report.pdf", "rb") as f:
    pdf_bytes = f.read()
result = client.parse_file(content=pdf_bytes, filename="report.pdf")

# Multiple formats:
result = client.parse_file(
    path="/path/to/report.pdf",
    formats=["markdown", "plainText"],
)

# Page cap:
result = client.parse_file(
    path="/path/to/large-report.pdf",
    parsers=[{"type": "pdf", "maxPages": 20}],
)

print(result["markdown"])
print("Pages:", result["metadata"]["numPages"])
```

**Signature:**

```python
def parse_file(
    path: str | None = None,
    *,
    content: bytes | None = None,
    filename: str | None = None,
    formats: list[str] | None = None,
    json_schema: dict | None = None,
    parsers: list[str] | None = None,
    **kwargs,
) -> dict
```

Provide either `path` (file on disk) or `content` (raw bytes). `filename` defaults to the basename of `path`, or `"document.pdf"` when using `content=` without a name.

### TypeScript SDK — `client.parseFile()`

The TypeScript SDK takes the file as a `Uint8Array` (not a path). Read the file before calling.

```typescript
import { CrwClient } from "crw-sdk";
import { readFileSync } from "fs";

const client = new CrwClient(); // CRW_API_KEY from env

// Basic:
const bytes = new Uint8Array(readFileSync("report.pdf"));
const result = await client.parseFile(bytes, { filename: "report.pdf" });
console.log(result.markdown);

// With options:
const result2 = await client.parseFile(bytes, {
  filename: "report.pdf",
  formats: ["markdown", "plainText"],
  parsers: [{ type: "pdf", maxPages: 20 }],
});
console.log(result2.metadata.numPages);
```

**Signature:**

```typescript
parseFile(
  content: Uint8Array,
  opts?: ParseFileOptions,
): Promise<ParseResult>

interface ParseFileOptions {
  filename?: string;       // default: "document.pdf"
  formats?: string[];
  jsonSchema?: object;
  parsers?: string[];
  [key: string]: unknown;  // any other engine option passed through
}
```

**Python vs TypeScript asymmetry:** The Python SDK accepts either a `path` string or raw `content` bytes (your choice). The TypeScript SDK accepts only raw bytes (`Uint8Array`) — you must read the file before calling `parseFile`. This is intentional: TypeScript environments (Deno, edge runtimes, browsers) cannot always read from a filesystem path.

## MCP tool — `crw_parse_file`

When running CRW via MCP (e.g. in Claude Desktop or Cursor), the `crw_parse_file` tool is available. It accepts Base64-encoded PDF bytes — the MCP transport cannot carry raw binary.

**Tool definition (excerpt):**

```json
{
  "name": "crw_parse_file",
  "description": "Parse a local PDF (base64 in contentBase64) to markdown. No OCR: scanned PDFs return empty markdown with a warning.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "contentBase64": { "type": "string", "description": "Base64-encoded PDF bytes" },
      "filename":      { "type": "string", "description": "Original filename (optional)" },
      "formats": {
        "type": "array",
        "items": { "type": "string", "enum": ["markdown", "plainText", "links", "json", "summary"] }
      },
      "jsonSchema":    { "type": "object", "description": "JSON schema for LLM extraction" },
      "parsers": {
        "type": "array",
        "items": { "type": "string", "enum": ["pdf"] }
      },
      "maxLength": { "type": "integer", "description": "Max chars per content field; 0 = unbounded" }
    },
    "required": ["contentBase64"]
  }
}
```

**Example call from Python with the MCP transport:**

```python
import base64
from crw import CrwClient

# local mode: CRW_LOCAL=1, no HTTP server needed
client = CrwClient()  # subprocess mode

with open("report.pdf", "rb") as f:
    pdf_bytes = f.read()

# The SDK handles base64 encoding automatically in local mode:
result = client.parse_file(content=pdf_bytes, filename="report.pdf")
print(result["markdown"])
```

The Python and TypeScript SDKs **automatically** base64-encode the bytes when running in local/MCP mode. You do not call `crw_parse_file` directly — call `parse_file()` / `parseFile()` and the SDK chooses the transport.

## Crawl → Parse workflow {#crawl-parse-workflow}

A common pattern: crawl a documentation site, identify PDF links in the crawl output, then parse each PDF for full-text content.

```python
import requests
import time
import base64
from crw import CrwClient

API_KEY = "YOUR_API_KEY"
HEADERS = {"Authorization": f"Bearer {API_KEY}", "Content-Type": "application/json"}
BASE = "https://api.fastcrw.com"

# 1. Start a crawl, collecting links only.
job = requests.post(f"{BASE}/v1/crawl", headers=HEADERS, json={
    "url": "https://example.com/reports",
    "maxPages": 50,
    "formats": ["links"],
}).json()
job_id = job["id"]

# 2. Poll until done.
while True:
    status = requests.get(f"{BASE}/v1/crawl/{job_id}", headers=HEADERS).json()
    if status["status"] in ("completed", "failed"):
        break
    time.sleep(2)

# 3. Collect PDF URLs from crawl results.
pdf_urls = []
for page in status.get("data", []):
    for link in page.get("links", []):
        if link.lower().endswith(".pdf"):
            pdf_urls.append(link)

print(f"Found {len(pdf_urls)} PDFs")

# 4. Download and parse each PDF.
client = CrwClient()
for url in pdf_urls[:5]:  # start small
    pdf_resp = requests.get(url)
    pdf_resp.raise_for_status()

    result = client.parse_file(
        content=pdf_resp.content,
        filename=url.split("/")[-1],
        formats=["markdown"],
    )
    print(f"\n--- {url} ({result['metadata']['numPages']} pages) ---")
    print(result["markdown"][:500])
```

## Structured extraction from PDFs

Combine `/firecrawl/v2/parse` with `formats: ["json"]` and a `jsonSchema` to extract structured data from a PDF in one step. Requires an LLM configured on the engine.

```bash
curl -X POST https://api.fastcrw.com/firecrawl/v2/parse \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -F "file=@/path/to/contract.pdf;type=application/pdf" \
  -F 'options={
    "formats": ["json"],
    "jsonSchema": {
      "type": "object",
      "properties": {
        "parties":       { "type": "array", "items": { "type": "string" } },
        "effectiveDate": { "type": "string" },
        "value":         { "type": "string" }
      }
    }
  }'
```

Response:

```json
{
  "success": true,
  "data": {
    "json": {
      "parties": ["Acme Corp", "Beta LLC"],
      "effectiveDate": "2026-01-01",
      "value": "$120,000"
    },
    "metadata": { "numPages": 8, "sourceFilename": "contract.pdf" }
  }
}
```

## Error reference

| Status | Cause | Fix |
|---|---|---|
| `400` | Missing `file` part, or file does not begin with `%PDF-` | Ensure the form includes a `file` field containing a valid PDF |
| `400` | `options` is not valid JSON | Validate the `options` string before sending |
| `400` | `formats: ["json"]` without a configured LLM | Set `[extraction.llm]` in `config.toml` or pass `llmApiKey` |
| `413` | Body exceeds 50 MiB | Split the PDF or trim pages before upload |
| `422` | Corrupt, encrypted, or unreadable PDF | Verify the PDF opens locally and is not password-protected |
| `503` | Document parsing is disabled on this server | Set `[document] enabled = true` in `config.toml` |

## Self-hosted configuration

Document parsing is enabled by default. To tune it, add a `[document]` section to `config.toml`:

```toml
[document]
enabled              = true
max_pages            = 0          # 0 = no limit
max_upload_bytes     = 52428800   # 50 MiB (hard cap)
upload_concurrency   = 4          # simultaneous uploads buffered in memory
max_concurrent_parses = 8         # across all surfaces (URL, crawl, upload)
parse_timeout_ms     = 30000      # ms; 0 = no timeout
max_decompressed_bytes = 104857600  # 100 MiB decompression-bomb guard
sandbox              = false      # isolate each parse in a child process
```

`sandbox = true` is recommended for hosts that accept untrusted PDF uploads. It runs each parse in a child process with hard OS memory and CPU limits. Cost: ~1–3 ms spawn overhead per parse.

## When to use something else

- Use [Scrape](#scraping) when the document is a web page, not a binary file
- Use [Extract](#extract) when you already have a URL to a PDF (scrape fetches and parses automatically when the response is `application/pdf`)
- Use [Crawl](#crawling) when you need to discover PDFs across an entire site before parsing them
