# Recipe: Parse PDF Reports to Markdown + Extract Fields

**Goal:** Upload a PDF file to fastCRW and get clean markdown back. Optionally
add a `jsonSchema` to extract structured fields (title, date, totals, parties)
from the same PDF in one request — no extra OCR pipeline, no intermediate files.

**Target file used in this recipe:** A public SEC 10-K filing —
`https://www.sec.gov/Archives/edgar/data/320193/000032019324000123/aapl-20240928.htm`
— but any PDF on disk works. The examples use a local file
`annual_report.pdf` (replace with your own path).

**What you will build:**

```
annual_report.pdf  →  POST /firecrawl/v2/parse  →  { markdown, json, metadata }
```

**Prerequisites:**

```bash
pip install crw
export CRW_API_KEY="crw-..."
```

---

## Part 1 — PDF to Markdown

The simplest call: upload a PDF, get markdown back. Default when no `formats`
option is set is `["markdown"]`.

:::tabs
::tab{title="Python (SDK)"}
```python
import os
from crw import CrwClient

client = CrwClient()  # reads CRW_API_KEY from env

# From a file path — SDK reads the bytes and POSTs multipart for you.
result = client.parse_file(path="annual_report.pdf")

md = result["markdown"]
meta = result["metadata"]

print(f"Pages : {meta['numPages']}")
print(f"Source: {meta['sourceFilename']}")
print()
print(md[:600])
```
::tab{title="cURL"}
```bash
curl -s -X POST https://api.fastcrw.com/firecrawl/v2/parse \
  -H "Authorization: Bearer $CRW_API_KEY" \
  -F "file=@annual_report.pdf;type=application/pdf" \
  | jq '{ pages: .data.metadata.numPages, preview: .data.markdown[:300] }'
```
:::

**Expected response:**

```json
{
  "success": true,
  "data": {
    "markdown": "# Annual Report 2025\n\nRevenue grew 18% year-over-year...",
    "metadata": {
      "sourceURL": "upload://annual_report.pdf",
      "sourceFilename": "annual_report.pdf",
      "numPages": 42,
      "statusCode": 200,
      "proxyUsed": "basic",
      "scrapeId": "a1b2c3d4-..."
    }
  }
}
```

Key metadata fields for parse responses:

| Field | Description |
|---|---|
| `numPages` | Total pages in the document |
| `sourceFilename` | Filename you uploaded |
| `sourceURL` | Always `upload://<filename>` |

---

## Part 2 — Extract structured fields with `jsonSchema`

Add `formats: ["json"]` and a `jsonSchema` to pull typed fields out of the
document in one shot. The engine parses the PDF to text first, then runs LLM
extraction over that text using your schema. Requires an LLM configured on the
engine (or your cloud account has one).

:::tabs
::tab{title="Python (SDK)"}
```python
import os
from crw import CrwClient

client = CrwClient()  # reads CRW_API_KEY from env

SCHEMA = {
    "type": "object",
    "properties": {
        "companyName":    {"type": "string"},
        "fiscalYear":     {"type": "string"},
        "totalRevenue":   {"type": "string", "description": "Total revenue with currency symbol"},
        "netIncome":      {"type": "string"},
        "reportingPeriod": {"type": "string"},
        "auditor":        {"type": "string"},
    },
    "required": ["companyName", "fiscalYear", "totalRevenue"],
}

result = client.parse_file(
    path="annual_report.pdf",
    formats=["markdown", "json"],  # both: prose + structured fields
    json_schema=SCHEMA,
)

# markdown is available too
print("Pages:", result["metadata"]["numPages"])
print()
# Extracted structured fields
fields = result.get("json", {})
print(f"Company : {fields.get('companyName')}")
print(f"FY      : {fields.get('fiscalYear')}")
print(f"Revenue : {fields.get('totalRevenue')}")
print(f"Net Inc : {fields.get('netIncome')}")
print(f"Auditor : {fields.get('auditor')}")
```
::tab{title="cURL"}
```bash
curl -s -X POST https://api.fastcrw.com/firecrawl/v2/parse \
  -H "Authorization: Bearer $CRW_API_KEY" \
  -F "file=@annual_report.pdf;type=application/pdf" \
  -F 'options={
    "formats": ["markdown", "json"],
    "jsonSchema": {
      "type": "object",
      "properties": {
        "companyName":    { "type": "string" },
        "fiscalYear":     { "type": "string" },
        "totalRevenue":   { "type": "string" },
        "netIncome":      { "type": "string" },
        "auditor":        { "type": "string" }
      },
      "required": ["companyName", "fiscalYear", "totalRevenue"]
    }
  }' \
  | jq '.data.json'
```
:::

**Expected response (`.data` excerpt):**

```json
{
  "markdown": "# Apple Inc. Form 10-K\n\nFor the fiscal year ended...",
  "json": {
    "companyName": "Apple Inc.",
    "fiscalYear": "2024",
    "totalRevenue": "$391.0 billion",
    "netIncome": "$93.7 billion",
    "reportingPeriod": "October 2023 – September 2024",
    "auditor": "Ernst & Young LLP"
  },
  "metadata": {
    "numPages": 88,
    "sourceFilename": "annual_report.pdf"
  }
}
```

---

## Part 3 — Limit pages + strip Markdown client-side

For large documents where you only need the executive summary (first N pages),
pass a `parsers` directive with `maxPages`. To get whitespace-stripped prose
(e.g. as token-efficient LLM context), request `markdown` and strip the
Markdown syntax client-side — `/firecrawl/v2/parse` does not return a `plainText` field
(that field exists in the internal engine type but is not included in the
`V2Document` response shape).

:::tabs
::tab{title="Python (SDK)"}
```python
import os
import re
from crw import CrwClient

client = CrwClient()

result = client.parse_file(
    path="annual_report.pdf",
    formats=["markdown"],
    parsers=[{"type": "pdf", "maxPages": 10}],  # first 10 pages only
)

print(f"Pages parsed: {result['metadata']['numPages']}")
print()

md = result["markdown"] or ""

# Strip Markdown syntax to get clean plain text for LLM context.
plain = re.sub(r"#{1,6}\s*", "", md)          # headings
plain = re.sub(r"\*{1,2}(.+?)\*{1,2}", r"\1", plain)  # bold / italic
plain = re.sub(r"`{1,3}[^`]*`{1,3}", "", plain)        # inline / fenced code
plain = re.sub(r"\[([^\]]+)\]\([^)]+\)", r"\1", plain) # links
plain = re.sub(r"\n{3,}", "\n\n", plain).strip()        # excess blank lines

print(plain[:400])
```
::tab{title="cURL"}
```bash
# /firecrawl/v2/parse does not return a plainText field — request markdown and
# post-process with jq or sed to remove Markdown syntax.
curl -s -X POST https://api.fastcrw.com/firecrawl/v2/parse \
  -H "Authorization: Bearer $CRW_API_KEY" \
  -F "file=@annual_report.pdf;type=application/pdf" \
  -F 'options={"formats":["markdown"],"parsers":[{"type":"pdf","maxPages":10}]}' \
  | jq '{ pages: .data.metadata.numPages, markdown_preview: .data.markdown[:300] }'
```
:::

> **Note — `parsers=` accepts strings or objects.** The SDK type annotation is
> `parsers: list[str | dict] | None`, matching the server's `ParserSpec`
> deserializer.  Pass a bare string (`["pdf"]`) when no extra options are needed,
> or a parser-spec dict (`[{"type": "pdf", "maxPages": 10}]`) to set options such
> as `maxPages`.

---

## Part 4 — Parse from bytes in memory

When you download the PDF over HTTP rather than reading from disk, pass raw bytes
directly so you avoid writing a temp file.

```python
import os
import urllib.request
from crw import CrwClient

client = CrwClient()

# Download any publicly accessible PDF
PDF_URL = "https://www.w3.org/WAI/WCAG21/wcag21.pdf"
with urllib.request.urlopen(PDF_URL) as resp:
    pdf_bytes = resp.read()

result = client.parse_file(
    content=pdf_bytes,
    filename="wcag21.pdf",         # shown in metadata.sourceFilename
    formats=["markdown"],
)

print(f"Downloaded {len(pdf_bytes):,} bytes")
print(f"Pages: {result['metadata']['numPages']}")
print(result["markdown"][:400])
```

---

## Complete script

```python
"""
recipe_pdf.py — parse a PDF report and extract structured fields with fastCRW.
Run:     python recipe_pdf.py annual_report.pdf
Requires: pip install crw
Env:      CRW_API_KEY
"""
import os
import sys
from crw import CrwClient

PDF_PATH = sys.argv[1] if len(sys.argv) > 1 else "annual_report.pdf"

SCHEMA = {
    "type": "object",
    "properties": {
        "companyName":  {"type": "string"},
        "fiscalYear":   {"type": "string"},
        "totalRevenue": {"type": "string"},
        "netIncome":    {"type": "string"},
        "auditor":      {"type": "string"},
    },
    "required": ["companyName", "fiscalYear", "totalRevenue"],
}

client = CrwClient()  # CRW_API_KEY from env

result = client.parse_file(
    path=PDF_PATH,
    formats=["markdown", "json"],
    json_schema=SCHEMA,
)

meta   = result["metadata"]
fields = result.get("json", {})

print(f"File   : {meta['sourceFilename']}")
print(f"Pages  : {meta['numPages']}")
print()
print("--- Extracted fields ---")
for key, val in fields.items():
    print(f"  {key:<20} {val}")
print()
print("--- Markdown preview (first 500 chars) ---")
print(result["markdown"][:500])
```

---

## Options reference

The `options` multipart field accepts a JSON string with these keys:

| Field | Type | Default | Description |
|---|---|---|---|
| `formats` | `string[]` | `["markdown"]` | Output formats. See below. |
| `jsonSchema` | `object` | — | JSON Schema for LLM extraction. Requires `"json"` in `formats`. |
| `parsers` | `array` | auto | Parser directives (see [PDF Parsing — parsers[]](pdf-parsing.md#parsers)). |
| `summaryPrompt` | `string` | — | Custom prompt for `"summary"` format. |
| `maxContentChars` | `number` | — | Truncate each content field to this many characters. |

**Supported formats for `/firecrawl/v2/parse`:**

| Format | Description |
|---|---|
| `markdown` | Extracted text as Markdown (default) |
| `links` | Array of URLs found in the PDF |
| `json` | Structured fields via LLM + `jsonSchema` |
| `summary` | LLM-generated prose summary |

`plainText` is not returned by `/firecrawl/v2/parse` — the `V2Document` response shape
does not include that field. Strip Markdown syntax client-side if you need
plain text (see Part 3 above).

Renderer-dependent formats (`html`, `rawHtml`, `changeTracking`) are not
applicable to uploaded documents and return a warning if requested.

---

## Error reference

| Status | Cause | Fix |
|---|---|---|
| `400` | Missing `file` part | Include `file` in the multipart form |
| `400` | File does not start with `%PDF-` | Confirm the file is a real PDF, not renamed HTML/text |
| `400` | `formats: ["json"]` without an LLM configured | Set `[extraction.llm]` in `config.toml` or use the cloud |
| `413` | Body exceeds 50 MiB | Split the PDF or trim pages with `parsers.maxPages` before uploading |
| `422` | Corrupt, encrypted, or password-protected PDF | Verify the PDF opens locally and is not locked |
| `503` | Document parsing disabled on this server | Set `[document] enabled = true` in `config.toml` |

---

## Notes

**Text-layer only.** fastCRW performs text extraction from the PDF's embedded
text layer. Scanned (image-only) PDFs with no text layer return empty or
near-empty markdown. Check `numPages` vs the length of `markdown` to detect
this — if `numPages` is large but `markdown` is short, the PDF is likely scanned.

**50 MiB limit.** The route enforces a hard 50 MiB body cap server-side.
Requests above this size receive `413 Content Too Large` before the body
is fully read.

**Both modes.** `client.parse_file()` works in HTTP mode (cloud or self-hosted
server) and in local subprocess mode (`CRW_LOCAL=1`). In subprocess mode the
SDK base64-encodes the bytes and calls the `crw_parse_file` MCP tool
automatically — you do not change your call.

---

## See also

- [PDF Parsing reference](pdf-parsing.md) — full options, `parsers[]` config, self-hosted tuning
- [Scraping](scraping.md) — when the document is a web page, not a binary file
- [Extract](extract.md) — structured LLM extraction from URLs (no file upload)
- [Recipe: RAG Knowledge Base](recipe-rag.md) — scrape a docs site, chunk, embed, and query
