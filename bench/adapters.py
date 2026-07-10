#!/usr/bin/env python3
"""Adapters: map each existing bench script's NATIVE output into the shared
per-item record (schema.py). The 3 scripts are not rewritten — we read what
they already emit and translate.

Tracks wired here:
  * extraction — bench/server-runs/diagnose_3way.jsonl  (crw vs crawl4ai vs firecrawl)
                 This IS the W1a competitor connector: same URLs, same scorer,
                 our harness — a real paired delta, not pasted vendor numbers.
  * map        — bench/map_results.json (per-site buckets) → per-GOLD-URL judgments
  * scrape     — bench/results.json / per-item run_bench output → crw-vs-gold

Every adapter returns a list of schema.item_record dicts; the caller appends
them to items.jsonl under a run_id.
"""

from __future__ import annotations

import json
import os

from schema import item_record


def _status_from_native(ok: bool, error, md_len: int) -> str:
    if error == "timeout":
        return "timeout"
    if error:
        return "error"
    if not ok or md_len == 0:
        return "empty"
    return "ok"


def extraction_to_items(run_id: str, jsonl_path: str) -> list[dict]:
    """diagnose_3way.jsonl → extraction items (primary=recall, secondary=found).

    Native row: {url, tool, ok, error, latency_ms, md_len, truth_recall, truth_found}.
    Emits, per (system,url): a continuous `recall` row (the paired-delta primary)
    and a binary `found` row (the public 63.74% headline, secondary). Latency is
    emitted too, timeouts scored at the cap (never dropped — survivorship).
    """
    records: list[dict] = []
    with open(jsonl_path) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            r = json.loads(line)
            system = r["tool"]
            item = r["url"]
            status = _status_from_native(
                r.get("ok", False), r.get("error"), r.get("md_len", 0)
            )
            records.append(item_record(run_id, "extraction", item, system,
                                       "recall", r.get("truth_recall", 0.0), status))
            records.append(item_record(run_id, "extraction", item, system,
                                       "found", 1.0 if r.get("truth_found") else 0.0,
                                       status))
            # Latency: timeouts already recorded at cap by diagnose_3way; status
            # rides along but latency rows are consumed by the latency stat, which
            # scores timeouts at the cap rather than excluding them.
            records.append(item_record(run_id, "extraction", item, system,
                                       "latency_ms", r.get("latency_ms", 0.0), status))
    return records


def map_to_items(run_id: str, results_path: str) -> list[dict]:
    """map_results.json → per-GOLD-URL binary judgments for a recall bootstrap.

    map_recall.py scores recall = found/expected per bucket. For a bootstrap CI
    the item must be a URL, so we expand each bucket into `expected` Bernoulli
    trials: `found` ones = 1 (discovered), the rest = 0 (missed). Bootstrapping
    over these gives the CI on the recall proportion — item=URL, of which there
    are many (NOT over the 2 sites).
    """
    records: list[dict] = []
    with open(results_path) as f:
        rows = json.load(f)
    for site in rows:
        host = site.get("url", "")
        for label, b in site.get("buckets", {}).items():
            expected = b.get("expected") or 0
            found = min(b.get("found", 0), expected)
            for i in range(expected):
                val = 1.0 if i < found else 0.0
                item = f"{host}|{label}|{i}"
                records.append(item_record(run_id, "map", item, "crw", "recall", val))
    return records


def scrape_to_items(run_id: str, items_path: str) -> list[dict]:
    """Per-item scrape output (JSONL of run_bench Results) → crw-vs-gold items.

    Emits `truth_found` (binary recall on matchable rows), `latency_ms`, and a
    `success` row. Rows where the dataset had no matchable truth_text are marked
    status=empty so they drop from the recall denominator (a bench artifact, not
    a scraper failure) — matching run_bench's own fair-recall rule.
    """
    records: list[dict] = []
    with open(items_path) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            r = json.loads(line)
            item = r["url"]
            if r.get("error") == "timeout":
                status = "timeout"
            elif not r.get("success"):
                status = "error" if r.get("error") else "empty"
            elif not r.get("truth_matchable"):
                status = "empty"  # no gold to score against
            else:
                status = "ok"
            records.append(item_record(run_id, "scrape", item, "crw", "truth_found",
                                       1.0 if r.get("truth_found") else 0.0, status))
            lat_status = "timeout" if r.get("error") == "timeout" else (
                "ok" if r.get("success") else "error")
            records.append(item_record(run_id, "scrape", item, "crw", "latency_ms",
                                       r.get("latency_ms", 0.0), lat_status))
    return records


def _selfcheck() -> int:
    import tempfile

    # Synthetic diagnose_3way.jsonl fixture (server-runs is empty until the 3-svc
    # rig runs). Exercises status mapping + the two emitted metrics per system.
    rows = [
        {"url": "http://a", "tool": "crw", "ok": True, "error": None,
         "md_len": 500, "latency_ms": 900, "truth_recall": 0.8, "truth_found": True},
        {"url": "http://a", "tool": "firecrawl", "ok": True, "error": None,
         "md_len": 400, "latency_ms": 1200, "truth_recall": 0.5, "truth_found": True},
        {"url": "http://b", "tool": "crw", "ok": False, "error": "timeout",
         "md_len": 0, "latency_ms": 60000, "truth_recall": 0.0, "truth_found": False},
    ]
    with tempfile.TemporaryDirectory() as d:
        p = os.path.join(d, "diagnose_3way.jsonl")
        with open(p, "w") as f:
            for r in rows:
                f.write(json.dumps(r) + "\n")
        recs = extraction_to_items("run1", p)
    # 3 native rows × 3 metrics (recall, found, latency_ms) = 9 records.
    assert len(recs) == 9, len(recs)
    rec_a_crw = [r for r in recs if r["item_id"] == "http://a" and r["system"] == "crw"]
    assert {r["metric"] for r in rec_a_crw} == {"recall", "found", "latency_ms"}
    recall_row = next(r for r in rec_a_crw if r["metric"] == "recall")
    assert recall_row["value"] == 0.8 and recall_row["status"] == "ok"
    timeout_row = next(r for r in recs if r["item_id"] == "http://b"
                       and r["metric"] == "recall")
    assert timeout_row["status"] == "timeout", timeout_row

    # map expansion: 3 expected, 2 found → two 1.0 and one 0.0.
    with tempfile.TemporaryDirectory() as d:
        p = os.path.join(d, "map_results.json")
        with open(p, "w") as f:
            json.dump([{"url": "https://s/", "buckets": {
                "products": {"found": 2, "expected": 3}}}], f)
        mrecs = map_to_items("run1", p)
    vals = sorted(r["value"] for r in mrecs)
    assert vals == [0.0, 1.0, 1.0], vals
    assert all(r["track"] == "map" and r["metric"] == "recall" for r in mrecs)

    print("adapters.py selfcheck OK — extraction/map/scrape → items")
    return 0


if __name__ == "__main__":
    raise SystemExit(_selfcheck())
