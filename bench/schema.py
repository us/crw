#!/usr/bin/env python3
"""Run orchestration + the shared per-item record every track appends to.

One `run_id` ties a whole benchmark run together; `manifest.json` (the
BenchmarkSpec) records the exact conditions so `report.py` can refuse to compare
rows from different harnesses. Every track writes the SAME per-item record to
`bench/runs/<run_id>/items.jsonl`; adapter functions (see adapters.py) map each
existing script's native output into this shape — the 3 scripts are NOT rewritten.

Record: {schema_v, run_id, track, item_id, system, metric, value, status}
  status ∈ ok | timeout | error | empty
"""

from __future__ import annotations

import hashlib
import json
import os
import time
import uuid
from dataclasses import asdict, dataclass, field

SCHEMA_V = 1
STATUSES = {"ok", "timeout", "error", "empty"}

RUNS_DIR = "bench/runs"


def mint_run_id() -> str:
    """A collision-proof run id: sortable timestamp + random suffix.

    A bare second-resolution timestamp collides when two tracks start in the
    same second; the uuid suffix removes that.
    """
    return time.strftime("%Y%m%d-%H%M%S") + "-" + uuid.uuid4().hex[:8]


def content_hash(data) -> str:
    """SHA-256 of a str/bytes or of the canonical-JSON of any JSON value."""
    if isinstance(data, bytes):
        raw = data
    elif isinstance(data, str):
        raw = data.encode("utf-8")
    else:
        raw = json.dumps(data, sort_keys=True, ensure_ascii=False).encode("utf-8")
    return hashlib.sha256(raw).hexdigest()


def item_record(
    run_id: str,
    track: str,
    item_id: str,
    system: str,
    metric: str,
    value: float,
    status: str = "ok",
) -> dict:
    if status not in STATUSES:
        raise ValueError(f"bad status {status!r}; must be one of {sorted(STATUSES)}")
    return {
        "schema_v": SCHEMA_V,
        "run_id": run_id,
        "track": track,
        "item_id": item_id,
        "system": system,
        "metric": metric,
        "value": float(value),
        "status": status,
    }


def run_dir(run_id: str) -> str:
    return os.path.join(RUNS_DIR, run_id)


def append_items(run_id: str, records: list[dict]) -> str:
    """Append records to bench/runs/<run_id>/items.jsonl."""
    d = run_dir(run_id)
    os.makedirs(d, exist_ok=True)
    path = os.path.join(d, "items.jsonl")
    with open(path, "a") as f:
        for r in records:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    return path


def read_items(run_id: str) -> list[dict]:
    path = os.path.join(run_dir(run_id), "items.jsonl")
    if not os.path.exists(path):
        return []
    with open(path) as f:
        return [json.loads(line) for line in f if line.strip()]


@dataclass
class Manifest:
    """The BenchmarkSpec — every track validates it before writing."""

    run_id: str
    git_sha: str
    seed: int
    temp: float = 0.0
    numpy_version: str = ""
    prereg_hash: str = ""
    # Hashes that make a comparison same-harness (report.py enforces a match):
    dataset_ids: dict[str, str] = field(default_factory=dict)  # {track: dataset_id}
    dataset_content_hashes: dict[str, str] = field(default_factory=dict)  # {track: hash}
    url_normalization_hash: str = ""
    primary_metric_per_track: dict[str, str] = field(default_factory=dict)
    comparison_systems: dict[str, list[str]] = field(default_factory=dict)  # {track: [systems]}
    answer_model: str = ""
    answer_provider: str = ""
    judge_model: str = ""
    judge_provider: str = ""
    search_provider: str = ""

    def path(self) -> str:
        return os.path.join(run_dir(self.run_id), "manifest.json")

    def write(self) -> str:
        os.makedirs(run_dir(self.run_id), exist_ok=True)
        p = self.path()
        with open(p, "w") as f:
            json.dump(asdict(self), f, indent=2, sort_keys=True)
        return p

    def harness_fingerprint(self, track: str) -> tuple:
        """The tuple that must match for two rows to be comparable (same-harness)."""
        return (
            self.dataset_content_hashes.get(track, ""),
            self.url_normalization_hash,
            f"{self.judge_model}@{self.judge_provider}",
            self.search_provider,
            self.prereg_hash,
        )


def read_manifest(run_id: str) -> Manifest:
    with open(os.path.join(run_dir(run_id), "manifest.json")) as f:
        return Manifest(**json.load(f))


def validate_manifest(m: Manifest) -> None:
    """Fail loudly BEFORE a track writes if the spec is under-specified."""
    if not m.run_id or not m.git_sha:
        raise ValueError("manifest missing run_id/git_sha")
    if m.temp != 0.0:
        raise ValueError(f"judge/answer temp must be pinned 0.0, got {m.temp}")
    if not m.prereg_hash:
        raise ValueError("manifest missing prereg_hash — commit bench/PREREG.md first")


# --- Missing-pair policy (pre-registered; §2) ------------------------------
def partition_pairs(
    crw: dict, competitor: dict
) -> tuple[dict, dict, list[str], dict]:
    """Split two {item_id: (value,status)} maps into paired-ok values + excluded.

    QUALITY metrics only: an item that is timeout|error|empty for EITHER system
    is excluded from the paired delta (not scored 0 — that games the gate).
    Returns (crw_ok, comp_ok, excluded_ids, failure_rates). Latency must NOT use
    this — a timeout is infinite latency, scored at the cap, never dropped.
    """
    ids = set(crw) & set(competitor)
    crw_ok, comp_ok, excluded = {}, {}, []
    fails = {"crw": 0, "competitor": 0}
    for i in sorted(ids):
        cv, cs = crw[i]
        kv, ks = competitor[i]
        if cs != "ok":
            fails["crw"] += 1
        if ks != "ok":
            fails["competitor"] += 1
        if cs == "ok" and ks == "ok":
            crw_ok[i] = cv
            comp_ok[i] = kv
        else:
            excluded.append(i)
    n = len(ids) or 1
    failure_rates = {k: round(v / n, 4) for k, v in fails.items()}
    failure_rates["excluded"] = len(excluded)
    return crw_ok, comp_ok, excluded, failure_rates


def worst_case_imputation(crw_ok: dict, comp_ok: dict, excluded_ids: list[str],
                          losing_value: float = 0.0, winning_value: float = 1.0):
    """Sensitivity bound: re-score with excluded items imputed as losses for crw.

    Guards against an asymmetric drop-out gaming the gate — if crw quietly gives
    up on hard pages, excluding them flatters it. This imputes every excluded
    item as crw's worst case (crw=losing, competitor=winning) and returns the
    padded delta arrays for a second bootstrap the report displays beside the
    primary one.
    """
    ids = sorted(set(crw_ok) | set(excluded_ids))
    crw_pad, comp_pad = [], []
    for i in ids:
        if i in crw_ok:
            crw_pad.append(crw_ok[i])
            comp_pad.append(comp_ok[i])
        else:
            crw_pad.append(losing_value)
            comp_pad.append(winning_value)
    return crw_pad, comp_pad


def _selfcheck() -> int:
    assert content_hash("abc") == content_hash("abc")
    assert content_hash({"a": 1, "b": 2}) == content_hash({"b": 2, "a": 1})  # key-order stable
    r = item_record("run1", "extraction", "u1", "crw", "recall", 0.7)
    assert r["schema_v"] == SCHEMA_V and r["status"] == "ok"
    try:
        item_record("run1", "t", "u1", "crw", "m", 0.5, status="bogus")
        raise AssertionError("bad status should raise")
    except ValueError:
        pass
    # missing-pair: crw empty on u2 → excluded, failure attributed to crw.
    crw = {"u1": (0.8, "ok"), "u2": (0.0, "empty"), "u3": (0.6, "ok")}
    comp = {"u1": (0.4, "ok"), "u2": (0.5, "ok"), "u3": (0.7, "ok")}
    co, ko, exc, fr = partition_pairs(crw, comp)
    assert exc == ["u2"] and co == {"u1": 0.8, "u3": 0.6}, (exc, co)
    assert fr["crw"] == round(1 / 3, 4) and fr["competitor"] == 0.0, fr
    cp, kp = worst_case_imputation(co, ko, exc)
    assert 0.0 in cp and len(cp) == 3, (cp, kp)  # excluded item imputed as crw loss
    two_uniq = mint_run_id() != mint_run_id()
    assert two_uniq, "run ids must be unique"
    manifest_ok_temp_guard()
    print("schema.py selfcheck OK — record/hash/manifest/missing-pair")
    return 0


def manifest_ok_temp_guard() -> None:
    m = Manifest(run_id="r", git_sha="abc", seed=1, prereg_hash="h")
    validate_manifest(m)  # ok
    m.temp = 0.7
    try:
        validate_manifest(m)
        raise AssertionError("nonzero temp must fail validation")
    except ValueError:
        pass


if __name__ == "__main__":
    raise SystemExit(_selfcheck())
