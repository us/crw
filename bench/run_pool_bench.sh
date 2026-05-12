#!/usr/bin/env bash
# Tier 5 — Browser-context pool bench gate.
#
# Runs the 1000-URL bench twice: once with the chrome browser-context pool
# DISABLED (baseline), once ENABLED (treatment). Captures bench JSON +
# Prometheus /metrics snapshots so the B2/B3a/B3b gate criteria from the
# plan can be evaluated by `compare_pool_bench.py`.
#
# Pre-reqs:
#   - docker compose stack ready (crw + chrome heavy profile)
#   - bench/.venv populated (`uv venv bench/.venv && uv pip install -r bench/requirements.txt`)
#   - HF_TOKEN exported (HuggingFace dataset access)
#
# Usage:
#   bench/run_pool_bench.sh
#   # → bench/server-runs/pool-<TS>-{off,on}.{json,log,metrics.txt}
#   # → bench/server-runs/pool-<TS>-summary.txt (gate evaluation)

set -euo pipefail
cd "$(dirname "$0")/.."

if [ -f .env ]; then set -a; . ./.env; set +a; fi

TS=$(date -u +%Y%m%d-%H%M%S)
OUT=bench/server-runs
mkdir -p "$OUT"

URLS="${BENCH_MAX_URLS:-1000}"
CONC="${BENCH_CONCURRENCY:-10}"
PORT="${CRW_PORT:-3030}"
COMPOSE_FILES=(-f docker-compose.yml -f docker-compose.override.yml)

run_pass() {
  local pool_enabled="$1"
  local label="$2"
  echo
  echo "============================================================"
  echo "PASS: pool=$label  (urls=$URLS, conc=$CONC)"
  echo "============================================================"

  CRW_RENDERER__MODE=chrome \
  CRW_RENDERER__CHROME_BACKEND=vanilla \
  CRW_RENDERER__CHROME_CONTEXT_POOL_ENABLED="$pool_enabled" \
    docker compose "${COMPOSE_FILES[@]}" --profile heavy up -d --force-recreate crw

  for i in {1..60}; do
    if curl -sf "http://localhost:$PORT/health" >/dev/null 2>&1; then
      echo "crw ready (port $PORT, pool=$label)"
      break
    fi
    sleep 2
  done

  json="$OUT/pool-$TS-$label.json"
  log="$OUT/pool-$TS-$label.log"
  metrics_pre="$OUT/pool-$TS-$label.metrics-pre.txt"
  metrics_post="$OUT/pool-$TS-$label.metrics-post.txt"

  curl -s "http://localhost:$PORT/metrics" > "$metrics_pre" || true

  CRW_API_URL="http://localhost:$PORT" \
  BENCH_CONCURRENCY="$CONC" \
  BENCH_MAX_URLS="$URLS" \
  BENCH_RESULTS_PATH="$json" \
    bench/.venv/bin/python bench/run_bench.py 2>&1 | tee "$log"

  curl -s "http://localhost:$PORT/metrics" > "$metrics_post" || true
  echo "→ saved $json + metrics snapshots"
}

run_pass false off
run_pass true  on

echo
echo "============================================================"
echo "Evaluating gate criteria..."
echo "============================================================"
bench/.venv/bin/python bench/compare_pool_bench.py \
  --off "$OUT/pool-$TS-off" \
  --on  "$OUT/pool-$TS-on" \
  | tee "$OUT/pool-$TS-summary.txt"
