# Tier 5 — Browser-Context Pool Bench Gate

This file is the operator runbook for the Tier 5 gate from
`plans/tamam-browser-ppolu-detaylica-virtual-glade.md`. It executes the
1000-URL bench twice — once with the pool disabled (baseline), once enabled
(treatment) — and checks the §B2 / §B3a / §B3b criteria.

## Pre-requisites

- Docker stack ready (`crw` + `chrome` heavy profile).
- `bench/.venv` set up:
  ```sh
  uv venv bench/.venv
  uv pip install --python bench/.venv/bin/python aiohttp datasets
  ```
- `HF_TOKEN` exported (HuggingFace dataset access for
  `firecrawl/scrape-content-dataset-v1`).
- ~30 minutes of wall-clock per pass (≈1 hour total for both passes).

## One-shot run

```sh
bench/run_pool_bench.sh
```

Outputs land in `bench/server-runs/`:

| File | Purpose |
|---|---|
| `pool-<TS>-off.json` | bench result, baseline |
| `pool-<TS>-off.log`  | bench stdout/stderr, baseline |
| `pool-<TS>-off.metrics-pre.txt`  | `/metrics` before baseline run |
| `pool-<TS>-off.metrics-post.txt` | `/metrics` after baseline run |
| `pool-<TS>-on.{json,log,metrics-{pre,post}.txt}` | same, treatment |
| `pool-<TS>-summary.txt` | gate evaluation (PASS/FAIL per criterion) |

## Gate criteria (auto-checked by `compare_pool_bench.py`)

| ID | Criterion | Source |
|---|---|---|
| **B2**  | `histogram_quantile(0.5, crw_chrome_request_handshake_seconds{pool="off"})` − `histogram_quantile(0.5, crw_chrome_request_handshake_seconds{pool="on", acquire_source="hit_idle"})` ≥ **300 ms** | warm-hit handshake savings |
| **B3a** | overall `pool="on"` median (across all `acquire_source`) ≤ `pool="off"` median | no operational regression |
| **B3b** | `truth_recall` within ±0.5 pp; `success_rate` ≥ baseline; request `p50` ≤ baseline + 100 ms | no scrape-quality regression |

`compare_pool_bench.py` exits with 0 on full PASS, 1 if any criterion fails.
Note that B3b reads from the bench JSON (latency_ms.p50, quality fields) and
B2/B3a from the `/metrics` snapshot — both must be present.

## After a PASS

Per plan §B5: flip `[renderer.chrome.pool] enabled = true` in
`config.docker.toml` only. Stealth tier stays off (browserless backend
unsupported in v1; see `[renderer.chrome] backend = "vanilla"`).

```diff
 # config.docker.toml
 [renderer.chrome.pool]
-enabled = false
+enabled = true
 # size = 4   # default = max(2, num_cpu/2)
```

Commit + ship via the normal flow. Roll back by flipping it back to `false`
and restarting — legacy WS-per-request path remains the fallback.

## After a FAIL

The `summary.txt` lists which criteria failed. Common patterns:

- **B2 reduction <300 ms**: pool may be sized wrong, or warm-hit ratio is
  too low (most acquires create new slots). Check the pool gauges
  `crw_chrome_pool_acquires_total{outcome=...}` ratio in the metrics
  snapshot. If `created_new` ≫ `hit_idle`, increase `pool.size`.
- **B3a fails but B2 passes**: warm-hits are fast but cold creates dominate
  the operational mix. Same fix — bigger pool, or look at
  `pool_recycle_seconds` for slow recycles.
- **B3b truth_recall regression**: real isolation bug. Stop, investigate
  with the T0/T1 cookie/storage tests in
  `crates/crw-renderer/tests/browser_pool_real_chrome.rs`. Do NOT flip the
  flag.

## Manual replay of just the gate evaluator

```sh
python3 bench/compare_pool_bench.py \
  --off bench/server-runs/pool-<TS>-off \
  --on  bench/server-runs/pool-<TS>-on
```

(No `.json` extension — the script appends suffixes.)
