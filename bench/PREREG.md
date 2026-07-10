# Benchmark pre-registration (P0 — multi-track discipline)

> Committed BEFORE any locked run. The whole point of the multi-track discipline
> is defensibility; a benchmark tuned against its own test set leaks. This file
> fixes, in advance, the ONE primary metric per track, the gate + threshold, the
> comparison systems, the URL-normalization rules, and the sample set + its hash.
> No post-hoc metric or competitor cherry-picking. If tuning happened, it happened
> on a separate dev set and is disclosed.
>
> The manifest of every run records `prereg_hash = sha256(this file)`; the report
> refuses to compare rows whose manifest hash differs.

## Global rules

- **Freeze before tune.** Gold/test sets are hashed and committed before any
  config or URL-normalization change. A separate dev set is used for iteration;
  the locked test set is scored once, at the end.
- **CI method fixed by sample size alone:** BCa iff `n < 300`, percentile
  otherwise. Never chosen post-hoc off observed skew.
- **Paired gate (quality, [0,1] metrics), deliberately asymmetric:**
  - `CI_low ≥ floor` → **WIN** (needs a practical floor — no overclaiming)
  - `CI_high ≤ 0` → **BEHIND** (any significant loss, no floor — losses surfaced)
  - else → **INCONCLUSIVE** ("threshold not established", not "no difference")
- **Missing-pair (quality only):** an item that is `timeout|error|empty` for
  either system is excluded from the paired delta (never scored 0). Per-system
  failure-rate is reported beside the delta, plus a worst-case-imputation bound
  (excluded items re-scored as crw losses). **Latency is exempt** — a timeout is
  infinite latency, scored at the cap, never dropped.
- **The CI reflects item-sampling noise only**, not judge/label error.
- **Bootstrap is the sole gate driver** for binary and continuous alike. Seed
  `12345`, paired `B=10,000`. numpy version pinned in the manifest.

## Per-track registration

| Track | Primary metric | Gate / floor | Comparison systems | Gold + hash |
|---|---|---|---|---|
| **relevance** | `recall@10` (binary, Wikipedia gold) | WIN floor `+0.05`; single-system-vs-gold, paired only if a relevance competitor connector lands | crw (+ Firecrawl iff connector built) | FRAMES `wiki_links`, frozen + hashed (W5) |
| **answer** | judge PASS rate (end-to-end) | WIN floor `+0.05` | crw (+ Firecrawl iff connector built) | FRAMES `test.tsv`, content hash in manifest |
| **extraction** | `recall` (continuous truth-recall) | WIN floor `+0.05`; `found`=recall≥0.3 is **secondary** | crw vs crawl4ai vs firecrawl (`diagnose_3way`, same URLs, our scorer) | firecrawl/scrape-content-dataset-v1, first N URLs, hashed |
| **scrape** | `truth_found` (binary, fair recall) | absolute crw-vs-gold (no competitor connector this pass → no paired verdict) | crw | firecrawl/scrape-content-dataset-v1 1000-URL, hashed |
| **map** | `recall` (per-GOLD-URL Bernoulli) | absolute crw-vs-gold; single-system bootstrap over per-URL judgments (item = URL, not the 2 sites) | crw | committed sandbox fixtures (books/quotes toscrape) |

## Latency ("faster" claim) — secondary, its own stat

- Fixed concurrency, warmup discarded, same network window.
- **Primary stat = ONE pre-registered percentile (p50)**, computed as a bootstrap
  CI on `median(latency_crw[i] − latency_comp[i])` — the median OF the per-URL
  paired deltas, NOT the difference of two medians.
- Timeouts scored at the cap (`60,000 ms`), never excluded. Per-system
  success-rate reported beside the number.
- p95/p99 are **context only** — tail-quantile bootstrap is unreliable at ~1k
  URLs; their intervals are flagged wide and do NOT gate.

## URL-normalization rules (relevance track)

`url-norm-v1` (hashed into the manifest): lowercase + strip `www.` host; force
`https`; urldecode; drop `#frag`; drop tracking params (`utm_*`, `gclid`,
`fbclid`, … — keep semantic params like `?q=`/`?id=`); strip trailing slash;
Wikipedia `/wiki/` path case-preserved with spaces↔underscores unified. Then
host+path (+kept query) exact match. See `bench/relevance.py`.

## Construct-validity acceptance rule (relevance)

`wiki_links` are Wikipedia authoring sources, not arbitrary web URLs — an answer
sourced off-Wikipedia scores as a miss, so the metric is **directional**. If the
Wikipedia-only gold proves too biased (pre-registered trigger: median per-query
`n_gold` < 2, OR > 30% of queries have empty `wiki_links`), switch to the
hand-labeled `n≈100` fallback set. This trigger is fixed here, not decided after
seeing scores.

## Multiplicity discipline

Reporting Recall/nDCG/MRR × {crawl4ai, firecrawl} and claiming a win on whichever
clears the gate inflates the false-win rate. The **primary** metric + comparison
are fixed above per track; everything else is reported as **secondary** context,
never used to declare the headline verdict.
