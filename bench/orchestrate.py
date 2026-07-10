#!/usr/bin/env python3
"""Run orchestration (§2) — mint a run_id, write the manifest, ingest each
track's native output into the shared items.jsonl, and render the report.

    # 1. start a run (writes bench/runs/<run_id>/manifest.json)
    python bench/orchestrate.py init --judge-model crw-pro --search-provider google-serp
    # 2. ingest an existing track output through its adapter
    python bench/orchestrate.py ingest <run_id> extraction bench/server-runs/diagnose_3way.jsonl
    python bench/orchestrate.py ingest <run_id> map bench/map_results.json
    # 3. run the live relevance track (needs a server + search backend)
    python bench/orchestrate.py relevance <run_id> --limit 100
    # 4. one honest report → bench/runs/<run_id>/report.md
    python bench/orchestrate.py report <run_id>

Adapters map native output → items.jsonl; the 3 legacy scripts are NOT rewritten.
"""

from __future__ import annotations

import argparse
import os
import subprocess

import numpy as np

import json

import adapters
import relevance
from report import build_report, losses
from schema import (
    Manifest,
    append_items,
    content_hash,
    mint_run_id,
    read_manifest,
    run_dir,
    validate_manifest,
)

PREREG_PATH = "bench/PREREG.md"

# Which primary metric each track is scored on (mirrors PREREG.md).
PRIMARY = {
    "relevance": "recall@k",
    "answer": "pass",
    "extraction": "recall",
    "scrape": "truth_found",
    "map": "recall",
}


def _git_sha() -> str:
    try:
        return subprocess.check_output(
            ["git", "rev-parse", "--short", "HEAD"], text=True).strip()
    except Exception:  # noqa: BLE001
        return "unknown"


def _prereg_hash() -> str:
    with open(PREREG_PATH, "rb") as f:
        return content_hash(f.read())


def cmd_init(a) -> int:
    run_id = mint_run_id()
    m = Manifest(
        run_id=run_id,
        git_sha=_git_sha(),
        seed=a.seed,
        numpy_version=np.__version__,
        prereg_hash=_prereg_hash(),
        url_normalization_hash=relevance.url_normalization_hash(),
        primary_metric_per_track=dict(PRIMARY),
        judge_model=a.judge_model,
        judge_provider=a.judge_provider,
        answer_model=a.answer_model or a.judge_model,
        answer_provider=a.answer_provider or a.judge_provider,
        search_provider=a.search_provider,
        comparison_systems={
            "extraction": ["crw", "crawl4ai", "firecrawl"],
            "scrape": ["crw"], "map": ["crw"], "relevance": ["crw"],
        },
    )
    validate_manifest(m)
    m.write()
    print(run_id)
    return 0


_ADAPTERS = {
    "extraction": adapters.extraction_to_items,
    "map": adapters.map_to_items,
    "scrape": adapters.scrape_to_items,
}


def cmd_ingest(a) -> int:
    read_manifest(a.run_id)  # fail if the run wasn't init'd
    if a.track not in _ADAPTERS:
        raise SystemExit(f"no adapter for track {a.track!r}; have {sorted(_ADAPTERS)}")
    recs = _ADAPTERS[a.track](a.run_id, a.native_path)
    append_items(a.run_id, recs)
    print(f"ingested {len(recs)} items into run {a.run_id} ({a.track})")
    return 0


GOLD_PATH = "bench/gold/frames_relevance_gold.jsonl"
GOLD_HASH_PATH = "bench/gold/frames_relevance_gold.sha256"


def cmd_relevance(a) -> int:
    m = read_manifest(a.run_id)
    ds = _load_frozen_gold()  # frozen BEFORE tuning; hash-verified below
    gold_hash = content_hash([d["gold"] for d in ds])
    committed = open(GOLD_HASH_PATH).read().strip()
    if gold_hash != committed:
        raise SystemExit(f"relevance gold drifted: {gold_hash} != committed {committed}")
    m.dataset_content_hashes["relevance"] = gold_hash
    m.write()
    # run_relevance keys off 'wiki_links'; feed it the pre-parsed frozen gold.
    items = [{"prompt": d["prompt"], "wiki_links": d["gold"]} for d in ds]
    out = relevance.run_relevance(a.run_id, items, limit=a.limit)
    append_items(a.run_id, out["records"])
    print(f"relevance: scored {out['scored']}, "
          f"excluded {out['excluded_empty_gold']} (empty gold), "
          f"{len(out['records'])} rows")
    return 0


def _load_frozen_gold() -> list[dict]:
    """Load the committed, hash-frozen relevance gold (freeze-before-tune)."""
    with open(GOLD_PATH) as f:
        return [json.loads(line) for line in f if line.strip()]


def cmd_report(a) -> int:
    from schema import read_items, read_manifest
    txt = build_report(a.run_id)
    d = run_dir(a.run_id)
    out = os.path.join(d, "report.md")
    with open(out, "w") as f:
        f.write(txt)
    # W4: persist the losses list (crw<competitor) for frozen-output re-scoring.
    loss = losses(read_items(a.run_id), read_manifest(a.run_id))
    with open(os.path.join(d, "losses.jsonl"), "w") as f:
        for row in loss:
            f.write(json.dumps(row) + "\n")
    print(f"wrote {out} (+ losses.jsonl: {len(loss)} crw losses)")
    if a.benchmarks:
        with open("BENCHMARKS.md", "w") as f:
            f.write(txt + "\n\n---\n_Prior point estimates (no intervals) are "
                    "superseded by this report._\n")
        print("regenerated BENCHMARKS.md")
    return 0


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest="cmd", required=True)

    pi = sub.add_parser("init")
    pi.add_argument("--seed", type=int, default=12345)
    pi.add_argument("--judge-model", default="crw-pro")
    pi.add_argument("--judge-provider", default="managed")
    pi.add_argument("--answer-model", default="")
    pi.add_argument("--answer-provider", default="")
    pi.add_argument("--search-provider", default="google-serp")
    pi.set_defaults(fn=cmd_init)

    pg = sub.add_parser("ingest")
    pg.add_argument("run_id")
    pg.add_argument("track")
    pg.add_argument("native_path")
    pg.set_defaults(fn=cmd_ingest)

    pr = sub.add_parser("relevance")
    pr.add_argument("run_id")
    pr.add_argument("--limit", type=int, default=None)
    pr.set_defaults(fn=cmd_relevance)

    prep = sub.add_parser("report")
    prep.add_argument("run_id")
    prep.add_argument("--benchmarks", action="store_true",
                      help="also regenerate BENCHMARKS.md")
    prep.set_defaults(fn=cmd_report)

    a = p.parse_args()
    return a.fn(a)


if __name__ == "__main__":
    raise SystemExit(main())
