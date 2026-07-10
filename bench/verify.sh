#!/usr/bin/env bash
# The ONE local verify command — runs the entire OFFLINE benchmark test suite
# (no server, no LLM, no network): stats, schema, adapters, relevance canon/IR
# metrics + edge cases, and the report same-harness guard. Gate this before any
# locked run. Bench is deliberately NOT in CI (network + LLM + live server).
set -euo pipefail
cd "$(dirname "$0")/.."
PY="${PY:-bench/.venv/bin/python}"
[ -x "$PY" ] || { echo "no venv at $PY — run: uv venv bench/.venv && uv pip install --python $PY -r bench/requirements.txt"; exit 1; }

echo "→ stats.py (paired bootstrap, 3-way gate, latency median)"; "$PY" bench/stats.py
echo "→ schema.py (record/hash/manifest/missing-pair)";          "$PY" bench/schema.py
echo "→ adapters.py (extraction/map/scrape → items)";            "$PY" bench/adapters.py
echo "→ relevance.py (canon/parse/recall/ndcg/mrr/edges)";       "$PY" bench/relevance.py
echo "→ report.py (same-harness guard + paired render)";         "$PY" bench/report.py --selfcheck
echo "→ map_recall.py --selfcheck (scorer baseline)";            "$PY" bench/map_recall.py --selfcheck >/dev/null && echo "  map scorer OK"
echo
echo "ALL OFFLINE BENCH CHECKS PASSED"
