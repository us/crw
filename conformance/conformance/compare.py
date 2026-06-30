"""Side-by-side: drive crw with the same corpus, diff each response shape
against the golden Firecrawl fixture, and print a compatibility scorecard.

    CRW_URL=http://localhost:3000 CRW_API_KEY=local uv run python -m conformance.compare

Exit non-zero if any Tier-1 case FAILs (<90% shape match), so CI gates on it.
"""

from __future__ import annotations

import json
import os
import pathlib

from . import corpus
from ._http import run_case
from .normalize import compare

CRW_URL = os.environ.get("CRW_URL", "http://localhost:3000")
KEY = os.environ.get("CRW_API_KEY", "")
FIXDIR = pathlib.Path(__file__).resolve().parent.parent / "fixtures" / "firecrawl_v2"
# Cases excluded from this run entirely (comma-separated names). CI sets this to
# `search_basic` because that case depends on a live SearXNG fanning out to
# third-party engines, which routinely rate-limit datacenter IPs and return zero
# results — making the gate flaky on a non-deterministic external dependency,
# not on crw. The search envelope's existing fields are covered by the Rust
# `tests/search_route.rs` unit tests; run `compare` locally with a SearXNG up
# (no skip) to exercise the live shape.
SKIP = {s.strip() for s in os.environ.get("CONFORMANCE_SKIP", "").split(",") if s.strip()}


def main() -> None:
    rows = []
    ok_fields = total_fields = 0
    for case in corpus.ALL_CASES:
        if case.name in SKIP:
            print(f"[skip-gate] {case.name}: excluded via CONFORMANCE_SKIP")
            continue
        fix = FIXDIR / f"{case.name}.json"
        if not fix.exists():
            print(f"[skip] {case.name}: no golden fixture (run capture first)")
            continue
        golden = json.loads(fix.read_text())["body"]
        status, body = run_case(CRW_URL, KEY, case)
        res = compare(golden, body)
        ok_fields += res["present"]
        total_fields += res["total"]
        flag = "PASS" if res["score"] == 100 else "WARN" if res["score"] >= 90 else "FAIL"
        rows.append((case.name, case.tier, flag))
        line = f"[{flag}] {case.name} (T{case.tier}) HTTP {status} shape {res['score']}%"
        if res["missing"]:
            line += f"  missing/mismatch: {res['missing']}"
        print(line)

    score = round(100 * ok_fields / (total_fields or 1), 1)
    print(f"\n=== compatibility scorecard: {score}% fields shape-match across {len(rows)} cases ===")

    t1_fail = [r[0] for r in rows if r[1] == 1 and r[2] == "FAIL"]
    if t1_fail:
        raise SystemExit(f"Tier-1 conformance FAILED: {t1_fail}")


if __name__ == "__main__":
    main()
