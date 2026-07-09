# crw-sdk

TypeScript/JavaScript SDK for [CRW](https://github.com/us/crw) — the fast,
Rust-native web data API. Use native `/v1` methods for new CRW integrations;
Firecrawl v2 compatibility is available separately for migration work.

Zero runtime dependencies (Node 18+ `fetch`). Dual ESM + CommonJS.

## Install

```bash
npm install crw-sdk
```

## Quick start — Cloud (default)

CRW is **cloud-first**. [Sign up for 500 free credits](https://fastcrw.com/dashboard)
— no payment, no monthly reset (GitHub/Google, ~10s) — then set `CRW_API_KEY`:

```ts
import { CrwClient } from "crw-sdk";

const crw = new CrwClient(); // reads CRW_API_KEY from the env
const res = await crw.scrape("https://example.com", { formats: ["markdown"] });
console.log(res.markdown);
```

```ts
// ...or pass the key explicitly
const crw = new CrwClient({ apiKey: "fc-..." });
```

## Self-hosting

```ts
// A self-hosted server:
const crw = new CrwClient({ apiUrl: "http://localhost:3000" });
```

```bash
# Local zero-config engine (no server, no key): set CRW_LOCAL=1.
# Requires the `crw-mcp` binary on PATH (or set CRW_BINARY).
CRW_LOCAL=1 node app.js
```

## Methods

| Method | Description | Mode |
|---|---|---|
| `scrape(url, opts?)` | Scrape one URL | both |
| `crawl(url, opts?)` | Crawl a site (async, polled) | both |
| `map(url, opts?)` | Discover URLs | both |
| `search(query, opts?)` | Web search (+ optional scrape) | both¹ |
| `parseFile(bytes, opts?)` | PDF → markdown / structured JSON | both |
| `extract({urls, schema?})` | Structured LLM extraction | HTTP |
| `batchScrape(urls, opts?)` | Scrape many URLs (async) | HTTP |
| `capabilities()` | Feature-detect the engine | HTTP |
| `changeTrackingDiff(cur, prev?)` | Diff vs a prior snapshot | HTTP |
| `close()` | Shut down the local subprocess | — |

¹ Local search needs a SearXNG URL configured on the engine.

```ts
// Structured extraction (async job → per-URL results array):
const results = await crw.extract({
  urls: ["https://example.com"],
  schema: { type: "object", properties: { title: { type: "string" } } },
});
// results: [{ url, status, data, error, llmUsage }]
for (const r of results) if (r.status === "completed") console.log(r.url, r.data);

// Parse a PDF:
import { readFileSync } from "node:fs";
const doc = await crw.parseFile(readFileSync("invoice.pdf"), { formats: ["markdown"] });
```

## Parity

This SDK mirrors the Python [`crw`](https://pypi.org/project/crw/) client method-for-method,
and both are conformance-tested against the engine's OpenAPI spec.
