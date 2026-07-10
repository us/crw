# fastCRW benchmark harness

Multi-track benchmark discipline: every published number carries a confidence
interval and a same-harness caveat, competitors run through OUR pipeline on
shared data (never pasted vendor numbers), and the gold sets are frozen before
tuning so the benchmark can't leak. See `plans/roadmap/phase-0-benchmark-discipline.md`
for the full rationale and `bench/PREREG.md` for the pre-registered choices.

## 0. Setup (stops this being tribal knowledge)

```bash
# Python env (no repo-wide pyproject; bench pins its own deps)
uv venv bench/.venv
uv pip install --python bench/.venv/bin/python -r bench/requirements.txt

# Prove the scorer works with ZERO setup (no server/LLM/network):
uv run python bench/map_recall.py --selfcheck        # → product recall ~0.71

# Run the whole OFFLINE test suite before any locked run:
bash bench/verify.sh
```

Live tracks need a server:
```bash
cargo run -p crw-cli -- serve                        # port 3000 (CRW_PORT/--port)
CRW_API_URL=http://localhost:3000 uv run python bench/map_recall.py
```
- **Port:** everything now honors `CRW_API_URL` (default `http://localhost:3000`,
  matching docker-compose's `3000:3000`). `diagnose_3way.py` no longer hardcodes 3030.
- **LLM** (judge + answer + extraction share ONE config): set
  `CRW_EXTRACTION__LLM__{PROVIDER,API_KEY,MODEL,BASE_URL}` on **both** the server
  and the bench process. There is no `OPENAI_API_KEY`/`ANTHROPIC_API_KEY` fallback.
- **Search / answer / relevance tracks** need SearXNG reachable: run it alongside
  the server so container DNS resolves — `docker compose up -d searxng crw` — or
  set `CRW_SEARCH__SEARXNG_URL`. A bare `cargo run -- serve` cannot reach the
  container by service name.
- **HF datasets:** `google/frames-benchmark` and `firecrawl/scrape-content-dataset-v1`
  download without a token here, but set `HF_TOKEN` if you hit rate limits.

## 1. The pieces

| File | Role |
|---|---|
| `PREREG.md` | Pre-registered primary metric / gate / systems / norm per track. Hashed into every manifest. |
| `stats.py` | The ONE home for cross-system math: paired bootstrap (B=10k), 3-way WIN/BEHIND/INCONCLUSIVE gate, single-system + latency-median bootstrap. `python bench/stats.py` self-checks. |
| `schema.py` | `run_id`, `manifest.json` (BenchmarkSpec), the shared per-item record, missing-pair policy. |
| `adapters.py` | Map each legacy script's native output → the shared `items.jsonl` (scripts NOT rewritten). |
| `relevance.py` | W2 relevance track: FRAMES `wiki_links` gold, URL canonicalization, Recall@10 / nDCG@10 / MRR. |
| `report.py` | One honest report: 5 separate track sections, no blended winner, same-harness guard. |
| `orchestrate.py` | Mint run, ingest tracks, run relevance, render report. |
| `verify.sh` | The single offline verify command. |

## 2. Run workflow

```bash
PY=bench/.venv/bin/python

# a) start a run → prints run_id, writes bench/runs/<run_id>/manifest.json
RID=$($PY bench/orchestrate.py init --judge-model crw-pro --search-provider google-serp)

# b) extraction paired delta (crw vs crawl4ai vs firecrawl) — the W1a connector.
#    Stand up the 3-service rig, run diagnose_3way, then ingest its per-item output:
$PY bench/diagnose_3way.py --max-urls 150            # writes bench/server-runs/diagnose_3way.jsonl
$PY bench/orchestrate.py ingest $RID extraction bench/server-runs/diagnose_3way.jsonl

# c) map track (single-system, per-URL bootstrap)
CRW_API_URL=http://localhost:3000 $PY bench/map_recall.py    # writes bench/map_results.json
$PY bench/orchestrate.py ingest $RID map bench/map_results.json

# d) relevance track (loads the frozen, hash-verified gold; needs server+search)
$PY bench/orchestrate.py relevance $RID --limit 200

# e) one honest report (+ regenerate BENCHMARKS.md)
$PY bench/orchestrate.py report $RID --benchmarks
#    → bench/runs/<run_id>/report.md, losses.jsonl; BENCHMARKS.md
```

## 3. Tracks

- **relevance** — single-system vs Wikipedia gold (`wiki_links`). Directional
  (Wikipedia-only construct validity; the switch-to-hand-labeled trigger is
  pre-registered). Frozen gold: `gold/frames_relevance_gold.jsonl` (+ `.sha256`).
- **answer** — FRAMES end-to-end judge PASS rate (the Rust `crw bench` harness).
- **extraction** — crw vs crawl4ai vs firecrawl, same URLs, our scorer. The real
  paired head-to-head (the 63.74% headline). Primary = continuous truth-recall;
  `found`=recall≥0.3 is secondary.
- **scrape** — crw-vs-gold on the 1000-URL set (no competitor connector this pass
  → absolute only).
- **map** — per-GOLD-URL Bernoulli judgments → single-system recall CI.

## 4. Discipline (the whole point)

- Freeze the gold + hash BEFORE tuning; the manifest records `prereg_hash`,
  dataset hashes, `url_normalization_hash`, seed, numpy version.
- Missing-pair (quality): exclude `timeout|error|empty` items (never score 0),
  report per-system failure-rate + a worst-case-imputation bound.
- Latency is EXEMPT from exclusion: a timeout is infinite latency, scored at the
  cap, never dropped. Primary latency stat = bootstrap CI on the median of
  per-URL paired deltas (p50); p95/p99 are context only.
- The CI reflects item-sampling noise only, not judge/label error.
- Bench is **not** in CI (network + LLM + live server); `verify.sh` is the local gate.
