#!/usr/bin/env python3
"""W3/W4 — one honest multi-track report from items.jsonl + manifest.json.

Renders ONE markdown with FIVE separate track sections (relevance, answer,
extraction, scrape, map) and NEVER a combined/overall score. Each published
number carries a CI and a same-harness caveat.

Same-harness guard (hardened): a comparison is refused unless every row shares
the run's manifest (one row per (system,track,item_id,metric) AND all rows carry
the manifest's run_id). Pasting a competitor's self-reported number is exactly
what this blocks; running a competitor through OUR pipeline on shared data (W1a)
is fine and is what produces these rows.
"""

from __future__ import annotations

import sys

import numpy as np

import stats
from schema import (
    Manifest,
    partition_pairs,
    read_items,
    read_manifest,
    worst_case_imputation,
)

TRACK_ORDER = ["relevance", "answer", "extraction", "scrape", "map"]
LATENCY_CAP_MS = 60_000  # timeouts scored here, never dropped
_CAVEAT = ("_CI reflects item-sampling noise only, not judge/label error. "
           "Same-harness: competitors run through our pipeline on shared data._")


def _key(r: dict) -> tuple:
    return (r["system"], r["track"], r["item_id"], r["metric"])


def assert_same_harness(records: list[dict], manifest: Manifest) -> None:
    """Refuse cross-harness / duplicated rows before any comparison."""
    seen = set()
    for r in records:
        if r["run_id"] != manifest.run_id:
            raise ValueError(
                f"cross-harness row: run_id {r['run_id']} != manifest {manifest.run_id}")
        k = _key(r)
        if k in seen:
            raise ValueError(f"duplicate row for {k} — one row per (system,track,item_id,metric)")
        seen.add(k)


def _by_metric(records: list[dict], track: str, metric: str) -> dict[str, dict]:
    """{system: {item_id: (value, status)}} for one track+metric."""
    out: dict[str, dict] = {}
    for r in records:
        if r["track"] == track and r["metric"] == metric:
            out.setdefault(r["system"], {})[r["item_id"]] = (r["value"], r["status"])
    return out


def _ok_values(m: dict[str, tuple]) -> list[float]:
    return [v for v, s in m.values() if s == "ok"]


def _fmt_ci(b: stats.BootResult) -> str:
    return f"{b.estimate:.4f}  (95% CI {b.ci_low:.4f}–{b.ci_high:.4f}, n={b.n}, {b.method})"


def _render_quality_track(track: str, records: list[dict], m: Manifest) -> str:
    metric = m.primary_metric_per_track.get(track, "recall")
    seed = m.seed
    floor = 0.05  # per-track override belongs in PREREG; default practical floor
    systems = _by_metric(records, track, metric)
    if "crw" not in systems:
        return f"### {track}\n_No crw rows for primary metric `{metric}`._\n"
    lines = [f"### {track}", f"Primary metric: `{metric}`  ·  seed {seed}", ""]

    # Absolute crw-vs-gold (always honest, even with no competitor).
    crw_abs = _ok_values(systems["crw"])
    if crw_abs:
        b = stats.bootstrap_single(crw_abs, seed)
        lines.append(f"- **crw (absolute):** {_fmt_ci(b)}")

    competitors = [s for s in systems if s != "crw"]
    if not competitors:
        lines.append("- _No competitor connector for this track → absolute only, "
                     "no paired verdict._")
    for comp in sorted(competitors):
        crw_ok, comp_ok, excluded, fails = partition_pairs(systems["crw"], systems[comp])
        if not crw_ok:
            lines.append(f"- **vs {comp}:** no overlapping OK items → no verdict.")
            continue
        deltas = [crw_ok[i] - comp_ok[i] for i in crw_ok]
        boot = stats.bootstrap_paired(deltas, seed)
        comp_mean = float(np.mean(list(comp_ok.values())))
        v = stats.verdict(boot, floor, comp_mean)
        rel = f", rel {v.relative_effect:+.1%}" if v.relative_effect is not None else ""
        lines.append(f"- **vs {comp}: {v.label}** — delta {_fmt_ci(boot)}{rel}")
        lines.append(f"    - failure-rate crw {fails['crw']:.1%} / {comp} "
                     f"{fails['competitor']:.1%}; excluded pairs {fails['excluded']}")
        # Worst-case sensitivity: excluded items imputed as crw losses.
        if excluded:
            cp, kp = worst_case_imputation(crw_ok, comp_ok, excluded)
            wboot = stats.bootstrap_paired([cp[i] - kp[i] for i in range(len(cp))], seed)
            wv = stats.verdict(wboot, floor)
            lines.append(f"    - worst-case (excluded→crw loss): {wv.label}, "
                         f"delta {wboot.estimate:+.4f} "
                         f"(CI {wboot.ci_low:.4f}–{wboot.ci_high:.4f})")

    # Latency (if this track emitted it): median of per-URL paired deltas,
    # timeouts scored at cap, NEVER excluded.
    lat = _by_metric(records, track, "latency_ms")
    if "crw" in lat and any(s != "crw" for s in lat):
        lines.append("")
        lines.append(_render_latency(lat, seed))
    lines.append("")
    lines.append(_CAVEAT)
    return "\n".join(lines) + "\n"


def _cap(v: float, status: str) -> float:
    return LATENCY_CAP_MS if status in ("timeout", "error") else v


def _render_latency(lat: dict[str, dict], seed: int) -> str:
    out = ["**Latency** (p50 = median of per-URL paired deltas, ms; timeouts at cap):"]
    crw = lat["crw"]
    for comp in sorted(s for s in lat if s != "crw"):
        ids = sorted(set(crw) & set(lat[comp]))
        if not ids:
            continue
        deltas = [_cap(*crw[i]) - _cap(*lat[comp][i]) for i in ids]
        bm = stats.bootstrap_median(deltas, seed)
        faster = "crw faster" if bm.estimate < 0 else "crw slower"
        crw_sr = sum(1 for i in ids if crw[i][1] == "ok") / len(ids)
        comp_sr = sum(1 for i in ids if lat[comp][i][1] == "ok") / len(ids)
        out.append(f"- vs {comp}: median Δ {bm.estimate:+.0f}ms "
                   f"(95% CI {bm.ci_low:+.0f}–{bm.ci_high:+.0f}, n={bm.n}) — {faster}; "
                   f"success crw {crw_sr:.0%} / {comp} {comp_sr:.0%}")
    out.append("_p95/p99 are context-only: tail-quantile bootstrap is unreliable "
               "at ~1k URLs and is not a gate._")
    return "\n".join(out)


def losses(records: list[dict], m: Manifest) -> list[dict]:
    """Items where crw lost to a competitor on the track's primary metric.

    W4 reproducibility: the losses list ships per run so a reviewer can re-score
    the frozen outputs (never re-run the LLM). Only OK-vs-OK pairs count.
    """
    out = []
    tracks = {r["track"] for r in records}
    for track in tracks:
        metric = m.primary_metric_per_track.get(track, "recall")
        systems = _by_metric(records, track, metric)
        if "crw" not in systems:
            continue
        for comp in (s for s in systems if s != "crw"):
            for item_id in set(systems["crw"]) & set(systems[comp]):
                cv, cs = systems["crw"][item_id]
                kv, ks = systems[comp][item_id]
                if cs == "ok" and ks == "ok" and cv < kv:
                    out.append({"track": track, "metric": metric, "item_id": item_id,
                                "competitor": comp, "crw": cv, "competitor_value": kv})
    return out


def build_report(run_id: str) -> str:
    m = read_manifest(run_id)
    records = read_items(run_id)
    assert_same_harness(records, m)
    tracks_present = {r["track"] for r in records}
    parts = [
        f"# fastCRW benchmark report — `{run_id}`",
        "",
        f"git `{m.git_sha}` · seed {m.seed} · numpy {m.numpy_version} · "
        f"judge `{m.judge_model}` · search `{m.search_provider}`",
        f"prereg `{m.prereg_hash[:12]}` · url-norm `{m.url_normalization_hash[:12]}`",
        "",
        "> Five independent tracks. **There is no blended/overall winner** — each "
        "track is scored on its own primary metric with its own interval.",
        "",
    ]
    for track in TRACK_ORDER:
        if track in tracks_present:
            parts.append(_render_quality_track(track, records, m))
        else:
            parts.append(f"### {track}\n_Not run in this benchmark._\n")
    return "\n".join(parts)


def _selfcheck() -> int:
    # Mixed-provenance guard: a foreign run_id row must be refused.
    m = Manifest(run_id="R", git_sha="abc", seed=7, prereg_hash="p",
                 primary_metric_per_track={"extraction": "recall"})
    good = [
        {"schema_v": 1, "run_id": "R", "track": "extraction", "item_id": "u1",
         "system": "crw", "metric": "recall", "value": 0.8, "status": "ok"},
        {"schema_v": 1, "run_id": "R", "track": "extraction", "item_id": "u1",
         "system": "firecrawl", "metric": "recall", "value": 0.4, "status": "ok"},
    ]
    assert_same_harness(good, m)  # ok
    dup = good + [good[0]]
    try:
        assert_same_harness(dup, m)
        raise AssertionError("duplicate key must be refused")
    except ValueError:
        pass
    foreign = good + [{**good[0], "run_id": "OTHER", "item_id": "u2"}]
    try:
        assert_same_harness(foreign, m)
        raise AssertionError("cross-harness run_id must be refused")
    except ValueError:
        pass

    # End-to-end: 60 paired extraction items, crw ahead by ~0.15 → WIN section.
    recs = []
    rng = np.random.default_rng(1)
    for i in range(60):
        cv = 0.7 + 0.05 * rng.standard_normal()
        recs.append({"schema_v": 1, "run_id": "R", "track": "extraction",
                     "item_id": f"u{i}", "system": "crw", "metric": "recall",
                     "value": cv, "status": "ok"})
        recs.append({"schema_v": 1, "run_id": "R", "track": "extraction",
                     "item_id": f"u{i}", "system": "firecrawl", "metric": "recall",
                     "value": cv - 0.15, "status": "ok"})
    txt = _render_quality_track("extraction", recs, m)
    assert "vs firecrawl: WIN" in txt, txt
    assert "no blended" not in txt  # that line lives in build_report, not the track
    print("report.py selfcheck OK — same-harness guard + paired render")
    return 0


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--selfcheck":
        raise SystemExit(_selfcheck())
    if len(sys.argv) > 1:
        print(build_report(sys.argv[1]))
    else:
        print("usage: report.py <run_id> | report.py --selfcheck", file=sys.stderr)
        raise SystemExit(2)
