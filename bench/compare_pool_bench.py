#!/usr/bin/env python3
"""Tier 5 — Browser-context pool bench gate evaluator.

Reads `pool-<TS>-{off,on}.{json,metrics-post.txt}` produced by
`run_pool_bench.sh` and prints PASS/FAIL for each plan §B2/B3a/B3b/B3b
criterion:

  B2  primary: histogram_quantile(0.5, ...) reduction on
      `crw_chrome_request_handshake_seconds{pool="on", acquire_source="hit_idle"}`
      vs `pool="off"` ≥ 0.300 s
  B3a secondary: overall median across all acquire_source values for pool=on
      ≤ pool=off median (no operational regression)
  B3b no-regression gates: truth_recall ±0.5 pp, success rate ≥ baseline,
      p50 ≤ baseline + 100 ms

Quantiles are computed in Python from raw `_bucket{le=...}` lines — Prometheus
isn't a hard dependency.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

HANDSHAKE = "crw_chrome_request_handshake_seconds"


def parse_buckets(text: str, metric: str, label_filter: dict[str, str] | None = None):
    """Yield (le_float, count_float) tuples for `metric_bucket` lines whose
    labels match every key/value in `label_filter`. Counts are cumulative
    (Prometheus convention)."""
    pat = re.compile(rf'^{re.escape(metric)}_bucket\{{([^}}]*)\}}\s+(\S+)$', re.M)
    for m in pat.finditer(text):
        labels_raw, count = m.group(1), m.group(2)
        labels = dict(re.findall(r'(\w+)="([^"]*)"', labels_raw))
        if label_filter:
            if any(labels.get(k) != v for k, v in label_filter.items()):
                continue
        le = labels.get("le")
        if le is None:
            continue
        try:
            le_f = float("inf") if le == "+Inf" else float(le)
            yield le_f, float(count)
        except ValueError:
            continue


def histogram_quantile(buckets: list[tuple[float, float]], q: float) -> float | None:
    """Linear-interpolation `histogram_quantile` matching Prometheus' formula
    on a single histogram. Returns None if the bucket list is empty / all-zero."""
    buckets = sorted(buckets, key=lambda x: x[0])
    if not buckets:
        return None
    total = buckets[-1][1]
    if total <= 0:
        return None
    target = q * total
    prev_le, prev_cum = 0.0, 0.0
    for le, cum in buckets:
        if cum >= target:
            if le == float("inf"):
                return prev_le
            in_bucket = cum - prev_cum
            if in_bucket <= 0:
                return prev_le
            frac = (target - prev_cum) / in_bucket
            return prev_le + frac * (le - prev_le)
        prev_le, prev_cum = le, cum
    return buckets[-1][0]


def aggregate_buckets(streams: list[list[tuple[float, float]]]) -> list[tuple[float, float]]:
    """Sum cumulative counts across multiple histograms keyed by `le`."""
    agg: dict[float, float] = {}
    for stream in streams:
        for le, cum in stream:
            agg[le] = agg.get(le, 0.0) + cum
    return sorted(agg.items(), key=lambda x: x[0])


def median_handshake(metrics_text: str, label_filter: dict[str, str] | None) -> float | None:
    """Sum `_bucket` series matching the filter (across whatever residual
    label dimensions exist) into one histogram, then `histogram_quantile(0.5)`."""
    by_le: dict[float, float] = {}
    for le, cum in parse_buckets(metrics_text, HANDSHAKE, label_filter):
        by_le[le] = by_le.get(le, 0.0) + cum
    if not by_le:
        return None
    return histogram_quantile(sorted(by_le.items(), key=lambda x: x[0]), 0.5)


def fmt_ms(s: float | None) -> str:
    return "n/a" if s is None else f"{s * 1000:.1f} ms"


def evaluate(off_prefix: Path, on_prefix: Path) -> int:
    off_json = json.loads(Path(f"{off_prefix}.json").read_text())
    on_json = json.loads(Path(f"{on_prefix}.json").read_text())
    off_metrics = Path(f"{off_prefix}.metrics-post.txt").read_text()
    on_metrics = Path(f"{on_prefix}.metrics-post.txt").read_text()

    failures: list[str] = []

    # ---- B2: warm-hit median reduction ≥ 300 ms ------------------------------
    off_med = median_handshake(off_metrics, {"pool": "off"})
    on_warm_med = median_handshake(on_metrics, {"pool": "on", "acquire_source": "hit_idle"})
    print("=" * 60)
    print("B2 — warm-hit handshake reduction")
    print(f"  pool=off median:                 {fmt_ms(off_med)}")
    print(f"  pool=on,acquire=hit_idle median: {fmt_ms(on_warm_med)}")
    if off_med is None or on_warm_med is None:
        failures.append("B2: missing histogram data (was the gate metric scraped?)")
        print("  → SKIP (missing data)")
    else:
        delta = off_med - on_warm_med
        print(f"  → reduction:                     {delta * 1000:.1f} ms (≥ 300 ms required)")
        if delta < 0.300:
            failures.append(f"B2: reduction {delta * 1000:.1f} ms < 300 ms")
            print("  → FAIL")
        else:
            print("  → PASS")

    # ---- B3a: overall pool=on median ≤ pool=off median -----------------------
    on_overall_med = median_handshake(on_metrics, {"pool": "on"})
    print()
    print("B3a — operational (all acquire_source) median ≤ baseline")
    print(f"  pool=off median:        {fmt_ms(off_med)}")
    print(f"  pool=on overall median: {fmt_ms(on_overall_med)}")
    if off_med is None or on_overall_med is None:
        failures.append("B3a: missing histogram data")
        print("  → SKIP (missing data)")
    elif on_overall_med > off_med:
        failures.append(
            f"B3a: pool=on overall median {on_overall_med * 1000:.1f} ms > "
            f"pool=off {off_med * 1000:.1f} ms"
        )
        print("  → FAIL")
    else:
        print("  → PASS")

    # ---- B3b: no-regression gates --------------------------------------------
    print()
    print("B3b — no-regression gates")
    off_recall = off_json["quality"]["truth_recall"]
    on_recall = on_json["quality"]["truth_recall"]
    print(f"  truth_recall: off={off_recall}%  on={on_recall}%  (±0.5 pp)")
    if abs(on_recall - off_recall) > 0.5:
        failures.append(f"B3b: truth_recall drift |{on_recall - off_recall:.2f}| > 0.5 pp")
        print("  → FAIL")
    else:
        print("  → PASS")

    off_succ = off_json["coverage"]["success_rate"]
    on_succ = on_json["coverage"]["success_rate"]
    print(f"  success_rate: off={off_succ}%  on={on_succ}%  (on ≥ off)")
    if on_succ < off_succ - 0.01:  # tiny epsilon for float noise
        failures.append(f"B3b: success_rate {on_succ}% < baseline {off_succ}%")
        print("  → FAIL")
    else:
        print("  → PASS")

    off_p50 = off_json["latency_ms"]["p50"]
    on_p50 = on_json["latency_ms"]["p50"]
    print(f"  request p50: off={off_p50} ms  on={on_p50} ms  (on ≤ off + 100 ms)")
    if on_p50 > off_p50 + 100:
        failures.append(f"B3b: request p50 {on_p50} ms > {off_p50 + 100} ms budget")
        print("  → FAIL")
    else:
        print("  → PASS")

    # ---- summary --------------------------------------------------------------
    print()
    print("=" * 60)
    if failures:
        print(f"GATE: FAIL — {len(failures)} criterion failed")
        for f in failures:
            print(f"  - {f}")
        return 1
    print("GATE: PASS — flip [renderer.chrome.pool] enabled = true in config.docker.toml")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description="Tier 5 pool bench gate evaluator")
    ap.add_argument("--off", required=True, help="Prefix for pool=off run (no extension)")
    ap.add_argument("--on", required=True, help="Prefix for pool=on run (no extension)")
    args = ap.parse_args()
    return evaluate(Path(args.off), Path(args.on))


if __name__ == "__main__":
    sys.exit(main())
