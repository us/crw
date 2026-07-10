#!/usr/bin/env python3
"""Cross-system benchmark statistics — the ONE home for paired-delta math.

Single-system CI already lives in Rust (`crw-cli/src/commands/bench.rs::bootstrap_ci`
for the pass rate). This module owns everything cross-system: the paired
bootstrap delta, the 3-way WIN/BEHIND/INCONCLUSIVE gate, and the single-system
bootstrap the map track needs so every published number carries an interval.

Design invariants (pre-registered — see bench/PREREG.md):
  * Paired bootstrap over per-item deltas d[i] = crw[i] - competitor[i].
  * CI method is fixed by SAMPLE SIZE alone: BCa iff n < N0 else percentile.
    Never pick BCa post-hoc off observed skew (that is a forking path).
  * The gate is deliberately asymmetric: WIN needs a practical floor
    (default +0.05, per-track in PREREG); BEHIND needs only any significant
    loss (no floor — we surface losses honestly).
  * The CI reflects item-sampling noise ONLY, not judge/label error.

Self-check: `python bench/stats.py` (or bench/test_stats.py) runs the offline
cases on fixed synthetic arrays — no server, no LLM, no network.
"""

from __future__ import annotations

from dataclasses import dataclass
from statistics import NormalDist

import numpy as np

# Pre-registered constants (do not tune off observed data).
DEFAULT_SEED = 12345
DEFAULT_B = 10_000  # paired-bootstrap resamples
SINGLE_B = 1_000  # single-system resamples (mirrors bench.rs)
N0 = 300  # BCa iff n < N0, percentile otherwise
DEFAULT_FLOOR = 0.05  # WIN practical floor; overridable per-track in PREREG

_NORM = NormalDist()

WIN = "WIN"
BEHIND = "BEHIND"
INCONCLUSIVE = "INCONCLUSIVE"


@dataclass
class BootResult:
    """A bootstrap interval and the point estimate it brackets."""

    estimate: float
    ci_low: float
    ci_high: float
    method: str  # "bca" | "percentile" | "degenerate"
    n: int


@dataclass
class Verdict:
    """The 3-way gate outcome for a paired comparison."""

    label: str  # WIN | BEHIND | INCONCLUSIVE
    boot: BootResult
    floor: float
    relative_effect: float | None  # delta / competitor_mean, guarded


def _resample_means(data: np.ndarray, b: int, rng: np.random.Generator) -> np.ndarray:
    n = len(data)
    idx = rng.integers(0, n, size=(b, n))
    return data[idx].mean(axis=1)


def _bca_interval(
    data: np.ndarray, theta_star: np.ndarray, alpha: float = 0.05
) -> tuple[float, float]:
    """Bias-corrected-and-accelerated percentiles (Efron & Tibshirani)."""
    theta_hat = float(data.mean())
    b = len(theta_star)
    # Bias correction z0 from the fraction of replicates below the estimate.
    prop = float(np.mean(theta_star < theta_hat))
    prop = min(max(prop, 1.0 / b), 1.0 - 1.0 / b)  # guard inv_cdf at 0/1
    z0 = _NORM.inv_cdf(prop)
    # Acceleration from jackknife leave-one-out means.
    n = len(data)
    total = data.sum()
    jack = (total - data) / (n - 1)  # leave-one-out means, vectorized
    jbar = jack.mean()
    diff = jbar - jack
    denom = 6.0 * (float(np.sum(diff**2)) ** 1.5)
    accel = float(np.sum(diff**3)) / denom if denom != 0 else 0.0
    lo_z, hi_z = _NORM.inv_cdf(alpha / 2), _NORM.inv_cdf(1 - alpha / 2)

    def adjust(z: float) -> float:
        p = z0 + (z0 + z) / (1 - accel * (z0 + z))
        return _NORM.cdf(p)

    a1, a2 = adjust(lo_z), adjust(hi_z)
    return (
        float(np.percentile(theta_star, 100 * a1)),
        float(np.percentile(theta_star, 100 * a2)),
    )


def bootstrap_paired(
    deltas: list[float] | np.ndarray,
    seed: int = DEFAULT_SEED,
    b: int = DEFAULT_B,
    n0: int = N0,
    method: str | None = None,
) -> BootResult:
    """Paired bootstrap 95% CI on mean(delta). Method fixed by n unless forced."""
    data = np.asarray(deltas, dtype=float)
    n = len(data)
    if n == 0:
        raise ValueError("bootstrap_paired: no paired items (empty delta list)")
    estimate = float(data.mean())
    # Degenerate: zero spread → the CI collapses to the point (constant array).
    if float(data.std()) == 0.0:
        return BootResult(estimate, estimate, estimate, "degenerate", n)
    chosen = method or ("bca" if n < n0 else "percentile")
    rng = np.random.default_rng(seed)
    theta_star = _resample_means(data, b, rng)
    if chosen == "bca":
        lo, hi = _bca_interval(data, theta_star)
    else:
        lo = float(np.percentile(theta_star, 2.5))
        hi = float(np.percentile(theta_star, 97.5))
    return BootResult(estimate, lo, hi, chosen, n)


def bootstrap_median(
    deltas: list[float] | np.ndarray, seed: int = DEFAULT_SEED, b: int = DEFAULT_B
) -> BootResult:
    """Bootstrap 95% CI on the MEDIAN of per-item paired deltas (latency primary).

    The pre-registered latency stat is median(latency_crw[i] - latency_comp[i]) —
    the median OF the per-URL deltas, NOT the difference of the two medians.
    Timeouts must already be scored at the cap by the caller (never excluded —
    survivorship bias). Percentile method (no floor gate; latency is descriptive).
    """
    data = np.asarray(deltas, dtype=float)
    n = len(data)
    if n == 0:
        raise ValueError("bootstrap_median: no paired items")
    estimate = float(np.median(data))
    if float(data.std()) == 0.0:
        return BootResult(estimate, estimate, estimate, "degenerate", n)
    rng = np.random.default_rng(seed)
    idx = rng.integers(0, n, size=(b, n))
    meds = np.median(data[idx], axis=1)
    return BootResult(
        estimate,
        float(np.percentile(meds, 2.5)),
        float(np.percentile(meds, 97.5)),
        "percentile-median",
        n,
    )


def bootstrap_single(
    values: list[float] | np.ndarray, seed: int = DEFAULT_SEED, b: int = SINGLE_B
) -> BootResult:
    """Single-system percentile CI (map track). Mirrors bench.rs bootstrap_ci."""
    data = np.asarray(values, dtype=float)
    n = len(data)
    if n == 0:
        raise ValueError("bootstrap_single: no values")
    estimate = float(data.mean())
    if float(data.std()) == 0.0:
        return BootResult(estimate, estimate, estimate, "degenerate", n)
    rng = np.random.default_rng(seed)
    theta_star = _resample_means(data, b, rng)
    return BootResult(
        estimate,
        float(np.percentile(theta_star, 2.5)),
        float(np.percentile(theta_star, 97.5)),
        "percentile",
        n,
    )


def verdict(boot: BootResult, floor: float = DEFAULT_FLOOR,
            competitor_mean: float | None = None) -> Verdict:
    """3-way gate. WIN iff CI_low >= floor; BEHIND iff CI_high <= 0; else INCONCLUSIVE."""
    if boot.ci_low >= floor:
        label = WIN
    elif boot.ci_high <= 0:
        label = BEHIND
    else:
        label = INCONCLUSIVE
    rel = None
    if competitor_mean is not None and abs(competitor_mean) >= 1e-9:
        rel = boot.estimate / competitor_mean
    return Verdict(label, boot, floor, rel)


def align_deltas(
    crw: dict[str, float], competitor: dict[str, float]
) -> tuple[list[float], list[str], list[str]]:
    """Align two {item_id: value} maps by key → (deltas, paired_ids, unpaired_ids).

    Only items present (and non-excluded) in BOTH maps are paired; the caller is
    responsible for having dropped timeout/error/empty items before calling
    (quality metrics only — latency scores timeouts at the cap, never drops).
    """
    paired_ids = sorted(set(crw) & set(competitor))
    unpaired = sorted(set(crw) ^ set(competitor))
    deltas = [crw[i] - competitor[i] for i in paired_ids]
    return deltas, paired_ids, unpaired


# ---------------------------------------------------------------------------
# Self-check — fixed synthetic arrays only (no RNG-drawn data → non-flaky).
# ---------------------------------------------------------------------------
def _selfcheck() -> int:
    # Center each fixed array on its exact target so the cases are deterministic
    # regardless of draw drift (input arrays are fixed; only resampling uses RNG).
    rng = np.random.default_rng(0)

    def centered(target: float, scale: float, n: int) -> np.ndarray:
        z = rng.standard_normal(n)
        return target + scale * (z - z.mean())

    # (a) known delta, large n → percentile method, CI width ≈ analytic 2·1.96·SE,
    #     and comfortably above the floor → WIN.
    a = centered(0.10, 0.02, 600)
    ba = bootstrap_paired(a, method="percentile")
    se = float(np.std(a, ddof=1)) / np.sqrt(len(a))
    analytic_w = 2 * 1.96 * se
    boot_w = ba.ci_high - ba.ci_low
    assert abs(boot_w - analytic_w) / analytic_w < 0.20, (boot_w, analytic_w)
    assert verdict(ba).label == WIN, ba

    # (b) null (mean 0) → straddles 0 → INCONCLUSIVE.
    b = centered(0.0, 0.05, 600)
    vb = verdict(bootstrap_paired(b, method="percentile"))
    assert vb.boot.ci_low < 0 < vb.boot.ci_high, vb.boot
    assert vb.label == INCONCLUSIVE, vb

    # (c) significant-but-trivial d≈0.02, tight → excludes 0 but below floor → INCONCLUSIVE.
    c = centered(0.02, 0.002, 600)
    vc = verdict(bootstrap_paired(c, method="percentile"))
    assert vc.boot.ci_low > 0, vc.boot  # a real, significant effect
    assert vc.boot.ci_low < 0.05, vc.boot  # but under the practical floor
    assert vc.label == INCONCLUSIVE, vc

    # (d) constant-array boundary: CI collapses to the point → pins the comparison logic.
    vd_lo = verdict(bootstrap_paired([0.04] * 50))
    vd_hi = verdict(bootstrap_paired([0.06] * 50))
    assert vd_lo.boot.method == "degenerate" and vd_lo.label == INCONCLUSIVE, vd_lo
    assert vd_hi.label == WIN, vd_hi  # 0.06 >= floor

    # (e) non-degenerate d centered 0.06 but with real spread reaching under the floor
    #     → a 0.06 POINT estimate is NOT a win → INCONCLUSIVE.
    e = centered(0.06, 0.10, 100)
    ve = verdict(bootstrap_paired(e))
    assert ve.boot.estimate > 0.05, ve.boot  # point estimate clears the floor
    assert ve.boot.ci_low < 0.05, ve.boot  # but the interval does not
    assert ve.label == INCONCLUSIVE, ve

    # (f) per-item JSONL fixture with a deliberate item_id misalignment → the join
    #     must pair by id, not by position (the most bug-prone path).
    crw = {"u3": 0.9, "u1": 0.5, "u2": 0.7}  # deliberately unsorted
    comp = {"u1": 0.4, "u2": 0.7, "u4": 0.1}  # u4 only in comp, u3 only in crw
    deltas, ids, unpaired = align_deltas(crw, comp)
    assert ids == ["u1", "u2"], ids
    assert deltas == [0.5 - 0.4, 0.7 - 0.7], deltas  # paired by id, u1 not by position
    assert unpaired == ["u3", "u4"], unpaired

    # (g) routing: n<N0 → bca, n>=N0 → percentile (pins the pre-registration rule).
    assert bootstrap_paired([0.06] * 0 + list(0.06 + 0.02 * rng.standard_normal(100))).method == "bca"
    assert bootstrap_paired(list(0.06 + 0.02 * rng.standard_normal(400))).method == "percentile"

    # (h) latency median: crw faster (deltas < 0) → median negative, CI excludes 0.
    lat = centered(-200.0, 50.0, 300)  # ms; crw 200ms faster per URL
    bm = bootstrap_median(lat)
    assert bm.estimate < 0 and bm.ci_high < 0, bm  # crw reliably faster

    print("stats.py selfcheck OK — cases a,b,c,d,e,f,g,h pass on fixed arrays")
    return 0


if __name__ == "__main__":
    raise SystemExit(_selfcheck())
