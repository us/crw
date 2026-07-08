# Benchmarks

3-way scrape benchmark on [Firecrawl's own public 1,000-URL dataset](https://huggingface.co/datasets/firecrawl/scrape-content-dataset-v1)
(`diagnose_3way.py`, 2026-05-08, concurrency 5, timeout 120s).

| Metric | **fastCRW** | crawl4ai | Firecrawl |
|---|---|---|---|
| Truth-recall (522/819 labeled URLs) <sup>recall mode</sup> | **63.74%** | 59.95% | 56.04% |
| p50 latency | **1914 ms** | 1916 ms | 2305 ms |
| p90 latency <sup>fast mode</sup> | **4348 ms** | 4754 ms | 6937 ms |
| Thrown errors (3,000 requests) | **0** | 0 | 0 |

fastCRW leads on every axis — top truth-recall, fastest median, lowest p90 tail — with
**0 thrown errors** across all 3,000 requests, and it uniquely recovers **34 URLs the other
two miss** (70% more than crawl4ai and Firecrawl combined). The 63.74% denominator is 819
labeled/matchable URLs, not 3,000 requests.

**Two modes, one engine, one config toggle.** *Recall mode* (the full ladder) maximizes
truth-recall, recovering the long tail of hard pages. *Fast mode* (LightPanda-only, no Chrome
tier) optimizes the latency tail — p90 4348 ms, the lowest of the three. Same binary, same
API; pick accuracy or latency per workload.

## How the two most-cited alternatives compare

| | **fastCRW** | Firecrawl | Crawl4AI |
|---|---|---|---|
| Language | Rust | Node.js + Playwright | Python + Playwright |
| License | AGPL-3.0 (commercial avail.) | AGPL-3.0 (commercial avail.) | Apache-2.0 |
| Self-host install size | Single binary (~8 MB) | Multi-container (~500 MB+) | ~2 GB (browser bundled) |
| Memory baseline (idle) | ~50 MB | Large (Chromium heap) | Large (Chromium heap) |
| Firecrawl migration | Yes — `/firecrawl/v2/*` compat layer | Native | No |
| MCP server | Built-in (`crw-mcp`) | Separate package | Community add-on |
| Hosted option | `api.fastcrw.com` | firecrawl.dev | None official |
| Reproducible public benchmark | Yes | Vendor-published only | Vendor-published only |

## Reproduce it yourself

The canonical harness is `bench/diagnose_3way.py` — it matches truth text against
`md + strip_md_links(md)`, applied identically to all three tools (a fairness control).

```bash
cd ~/coding/crw/crw-opencore
docker compose -f docker-compose.yml -f docker-compose.override.yml \
               -f docker-compose.stealth.yml --profile stealth up -d
docker start crawl4ai-bench
cd ~/coding/crw/competitors/firecrawl && docker compose up -d

cd ~/coding/crw/crw-opencore
uv run python bench/diagnose_3way.py \
  --max-urls 1000 --tools crw,crawl4ai,firecrawl \
  --concurrency 5 --timeout 120 \
  --out bench/server-runs/diag3w-1000-full.jsonl
```

Full result of record: [`bench/server-runs/RESULT_3WAY_1000_FULL.md`](bench/server-runs/RESULT_3WAY_1000_FULL.md).
Live dashboard: [fastcrw.com/benchmarks](https://fastcrw.com/benchmarks).
