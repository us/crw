# crw — Firecrawl v2 conformance suite (issue #62)

Proves crw's `/v2/*` API is compatible with Firecrawl v2 along two axes:

1. **SDK conformance (the literal #62 gate)** — the real `firecrawl-py` SDK,
   pointed at a self-hosted crw, runs its 6 core methods without a 404 and
   returns SDK-parseable objects.
2. **Golden-fixture shape diff** — crw's responses are diffed, field-by-field,
   against captured **real** Firecrawl v2 responses. Value-independent: it
   compares structure (keys + types), since content legitimately differs.

## Run

```bash
cd conformance

# 1. Diff crw against the committed golden fixtures + run the SDK suite (CI gate)
CRW_URL=http://localhost:3000 CRW_API_KEY=local ./run.sh all

# just the shape diff, or just the SDK:
CRW_URL=http://localhost:3000 ./run.sh compare
CRW_URL=http://localhost:3000 CRW_API_KEY=local ./run.sh sdk

# 2. Refresh golden fixtures from the LIVE Firecrawl API (drift check)
FIRECRAWL_API_KEY=fc-... ./run.sh capture
```

`uv` provides the environment (`firecrawl-py`). The Firecrawl API key is read
from `FIRECRAWL_API_KEY` and is **never committed** (`.gitignore`).

## Fixtures

`fixtures/firecrawl_v2/*.json` are the committed reference shapes — the contract
crw targets for this PR (the SDK-critical subset of the live v2 Document /
status / map / search envelopes, captured from `api.firecrawl.dev/v2`).

`./run.sh capture` overwrites them with the **full live** responses. Re-running
`compare` then surfaces upstream-only enrichments crw doesn't yet emit as
tracked Tier-2/3 gaps (e.g. `map` link `title`/`description`, scrape
`metadata.viewport`/`cachedAt`) — honest drift detection, not a hard failure.

## Corpus

`conformance/corpus.py` defines the deterministic request set (fixed target
URLs + the scrape format matrix: bare-string vs object formats, json+schema,
summary, multi-format). Async cases (crawl/batch/extract) start then poll to a
terminal status.
