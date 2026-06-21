# Quality-Neutral Latency Reduction (scrape + search) — Implementation Plan

> Branches `perf/latency-qn` in two worktrees:
> - opencore (Rust): `/Users/us/coding/crw/crw-opencore-latency`
> - saas (Node/TS): `/Users/us/coding/crw/crw-saas-latency`
>
> **Benchmark harness lives in a THIRD worktree:** `/Users/us/coding/crw/crw-opencore-p90/bench/`
> (`diagnose.py`, `p90_summary.py`, `P90_RESULTS.md`) and `/Users/us/coding/crw/search-bench/`.
> These are NOT present in the latency worktree — all bench commands `cd` into those paths.
> Before starting, confirm the harness builds/points at the crw binary built from this branch.

## Context

Lower scrape/search latency (p50/p90) under **three hard constraints**:
1. **Zero content-quality loss** — extracted markdown/text must not get thinner.
2. **Never return a failed/partial/incomplete result.**
3. **No caching** (unique traffic → ~0 hit rate; user already rejected — correct YAGNI).

Guiding principle: **cut wasted time, not honest work** — produce a byte-identical (or strictly
superior) result, only sooner.

> ⚠️ **Constraint-2 reality check (Codex finding, verified).** The SaaS search route ALREADY
> returns partial enrichment and refunds the un-delivered portion — `route.ts:503` (managed
> engine partial-scrape refund) and `route.ts:891` (non-LLM mode). So "never partial" is **already
> not true** for search enrichment today. This plan does NOT change that contract and does NOT make
> it worse; "never partial" is therefore scoped to mean **"this plan introduces no new partial/failed
> path, and the streaming work delivers the SAME (possibly-already-partial-with-refund) result the
> blocking path would have."** Fixing the pre-existing partial-enrichment contract is explicitly
> out of scope for this round (separate decision).

### Verified facts from research — what already exists (do NOT rebuild)

- **Resource blocking is implemented and ON** — `blocklist.rs` (`should_block(resource_type,url)`;
  default-blocked Image/Media/Font/Manifest/WebSocket; Stylesheet behind off-by-default flag; host
  substrings) + `cdp.rs:776-846` `run_intercept_pump` (Fetch.requestPaused → fail/continueRequest).
  Bench configs set `chrome_intercept_resources = true`. → not a new lever.
- **Content-ready detection exists** — `NetworkActivityTracker` (cdp.rs:968-1010, `is_idle(quiet_ms)`,
  `NETWORK_IDLE_QUIET_MS=500`) + `wait_for_spa_selector` (cdp.rs:1744-1795).
- **Hedged/racing rendering was tried and REVERTED** — `stealth.hedge.toml` documented as a net
  loss in `P90_RESULTS.md:128-143` (floods browserless, 63 fires / 11 consumed, inflates p90).
  **Blind hedging stays off the table.**
- **Escalation is strictly sequential; per-host promotion only REORDERS it** — `lib.rs:831-970`
  always runs HTTP first; `preference.rs` (`PROMOTION_THRESHOLD=3`/15-min window) latches a host to
  Chrome-first but never skips the HTTP leg. Preference state is **in-memory** (lost on restart).
- **The pump's own doc comment says serialization is already cheap** — `cdp.rs:771-775`: "the
  per-handler CDP roundtrip is sub-millisecond on a local socket … chrome queues paused requests
  internally." This directly undercuts the value of parallelizing it (see Phase F).
- **The bench harness already measures content quality** — `diagnose.py` computes `truth_recall`
  (firecrawl `scrape-content-dataset-v1` phrase matching, ≥20-char phrases), `lie_text` leak check,
  per-URL `markdown_len`; `p90_summary.py` emits p50/p90/p95/p99. Note: `compare_pool_bench.py`
  reads a different (pool-bench) JSON format and has **no per-URL markdown_len gate** — that gate
  must be written (Phase V).
- **SaaS search is fully blocking, single JSON** — `route.ts:421` one `await crwFetch`;
  `crw-client.ts` buffers `res.text()`; engine `search.rs:137-628` returns one `Json(resp)`.

### Baseline numbers (from `crw-opencore-p90/bench/P90_RESULTS.md`, **stealth rig**, N=100, conc 3, t/o 45s)

| Arm | recall | success | p50 | p90 | p99 |
|---|---|---|---|---|---|
| A0-stealth (baseline) | 67.57% | 89% | 2024ms | 15448ms | 37415ms |
| A1-stealth (shipped, spa_cap=3000) | 67.12% (−0.45pp) | — | — | 14406ms (−6.7%) | 31366ms (−16%) |

**The p90 tail (15.4s) and p99 (37.4s) are dominated by Chrome render on hard pages, NOT by
pre-render overhead.** This single fact reorders the whole plan (Codex + pragmatist consensus):
levers that don't touch Chrome render time cannot move p90.

## What we are NOT doing (scope guard)

- ❌ No response cache (rejected — unique traffic).
- ❌ No blind hedged/racing rendering (reverted; contends for the 2-cold-slot pool).
- ❌ No timer-trimming "fast-mode" as default (p90 branch's approach; A1's `spa_selector_max_ms=3000`
  is the already-shipped safe edge and is not re-litigated).
- ❌ No content-affecting blocking (never block document/script/xhr/fetch).
- ❌ No fixing the pre-existing partial-enrichment search contract (out of scope; see Context).
- ❌ No new partial/degraded responses. Streaming = progressive delivery of the SAME result the
  blocking path would have produced, with identical credit accounting.

---

## Phase 0 — Instrument & measure where p90 actually goes  *(opencore; PREREQUISITE, ships nothing user-facing)*

**Rationale (Codex + pragmatist consensus):** the v1 plan assumed where the time goes. Before
committing effort, measure it. Rank every later phase by *measured* p90 contribution; kill phases
whose target turns out to be sub-second.

- **Files:** `crates/crw-renderer/src/lib.rs`, `cdp.rs` (add structured timing logs / metrics).
- **Steps:** emit per-request structured timing: `http_leg_ms`, renderer chain + which tier
  produced the accepted result, `preference` hit/miss + skip-candidate, fallback reason,
  `paused_request_count`, intercept-pump queue delay + `Fetch.continue/fail` RTT, nav-wait bucket
  (selector-exit vs network-idle vs budget-exhausted), final `content_len`. Behind a
  `[telemetry] latency_breakdown = false` flag (default off; on for bench runs only).
- **Output:** run the stealth rig once with this on; produce a histogram of where p90/p99 requests
  spend their time (HTTP leg? Lightpanda? Chrome render? selector-budget-exhaustion? challenge?).
- **Decision gate:** this histogram **re-ranks Phases A–F**. If a phase's target is <300ms of the
  p90 budget, that phase is dropped before any code is written.
- **Effort:** S–M. **Risk:** 🔵 (measurement only).

---

## Real-latency phases (these target the actual p90/p99 tail)

### Phase A — Right-size the browser pool + Chrome launch flags for the 4-core box  *(opencore)*
- **Why:** the tail *may* be render capacity. The reverted hedge failed because it saturated a
  small pool. Right-sizing + leaner Chrome startup is quality-neutral (render output identical; only
  how many run and how fast they boot changes).
- **⚠️ Pool-config disambiguation (Codex finding — resolve in Phase 0 BEFORE touching anything):**
  there are TWO pool knobs and the plan must target the one actually limiting the stealth rig/prod:
  - legacy `pool_size` defaults to **4** (`config.rs:1011`);
  - the browser-context pool size is `None → max(2, n/2)` = **2 on a 4-core box** (`config.rs:741`,
    `lib.rs:439-444`).
  Phase 0 must log which limiter is active in the stealth rig and in prod, else Phase A benchmarks
  the wrong concurrency limiter.
- **⚠️ Queue-wait vs in-render (pragmatist finding — the A go/no-go discriminator):** pool sizing
  only helps if the tail is requests *waiting for a slot*. If the tail is a single hard SPA taking
  15-40s to render *once acquired*, more slots do nothing. **Phase 0 must separate "time waiting for
  a pool slot" from "time rendering after acquire."** If the tail is in-render, Phase A is NOT the
  lever and **the quality-neutral budget is likely exhausted** (further p90 cuts would need the
  nav-budget trim the user ruled out) — an honest, important negative result, not a failure.
- **Files:** `lib.rs`, `browser_pool.rs`, Chrome launch args (docker-compose `headless-shell`),
  `crw-core/src/config.rs`.
- **Steps:** (1) make the *active* pool size configurable; bench 2/3/4/6 on the stealth rig at
  prod-matching concurrency; (2) audit/add quality-neutral Chrome flags (`--disable-extensions`,
  `--disable-background-networking`, `--disable-features=Translate,BackForwardCache`, JS-heap cap)
  that cut boot/idle without changing rendered DOM; (3) measure cold-vs-warm acquire AND
  queue-wait-vs-render split.
- **Quality-safety:** none change the rendered DOM or extracted text. Pure capacity/boot.
- **Risk:** 🟡 over-sizing on 8GB can OOM stealth Chrome (heavy contexts) → cap by memory not cores,
  bench RAM headroom. **Prod-shadow is a BLOCKING exit gate for A** (local "size 6 wins" can OOM/
  regress on the 8GB Hetzner box). **Flag:** `[renderer] pool_size` (default = today's).
- **Effort:** M.

### Phase B — DNS caching + proxy-client-cache audit  *(opencore; RESCOPED — likely marginal)*
- **⚠️ Rescope (pragmatist finding — connection reuse is MOSTLY ALREADY DONE):** verified in code —
  the search client already holds `Arc<reqwest::Client>` "so the connection pool is hot"
  (`crw-search/src/client.rs:136-139`); the base HTTP fetcher is built once as `Arc<HttpFetcher>`
  (`lib.rs:301`); a `proxy_client_cache` already exists (`lib.rs:360,591`). The ONLY genuine gaps:
  (1) `with_proxy` builds a fresh client per request (`lib.rs:635`) for per-request proxy egress
  (BYOP/rotation) — that path **cannot** reuse connections by design, so no keep-alive win is
  available on the proxied (stealth) tail; (2) no resolver-level DNS cache. **This phase is likely
  a near-no-op after A and should not promise keep-alive gains on proxied traffic.**
- **Files:** `http_only.rs` (reqwest builder), `lib.rs` (`proxy_client_cache`), `crw-search/src/client.rs`.
- **Steps:** add a bounded resolver-level DNS cache (NOT response cache — the "no cache" rule does
  not apply to DNS; e.g. `hickory-resolver` or reqwest TTL) if absent; **audit `proxy_client_cache`
  hit rate** (Phase 0 telemetry) to see whether proxied traffic can share clients at all.
- **Quality-safety:** identical bytes; transport only.
- **Risk:** 🔵 DNS staleness (bounded TTL); 🔵 may benchmark as no-op → only ship if Phase 0 shows
  DNS resolution is a measurable share. **Flag:** `[net] dns_cache_ttl_s` (0=off, default 0).
- **Effort:** S.

### Phase C — Parallelize the answer-mode search pipeline  *(opencore)*
Split into two sub-phases after the Codex finding that early-synthesis has **no runnable quality
gate today** (search-bench measures domain overlap/snippet coverage, NOT answer-mode
`gold-in-sources` recall — `analyze.py:42-65`).

**Phase C1 — schedule overlap, IDENTICAL final source set (quality-neutral by construction).**
- **What:** overlap the page-2 fallback fetch with first-page scraping; overlap SearXNG-fetch with
  the start of enrichment where the final source set is unchanged. The LLM still synthesizes from
  the **exact same source set** the blocking path would use — only the *scheduling* changes, never
  *which* sources feed the answer.
- **Precondition (pragmatist):** only overlap a stage whose **input set is already final**. Page-2
  overlap is safe (page-1 enrichment targets are fixed before page-2 arrives). Do NOT start
  enrichment early if page-2 results could still change which URLs get enriched — that would change
  the source set. The source-set-equality test backstops this, but state it so the test isn't the
  only guard.
- **Quality-safety:** byte-identical answer inputs → proven-neutral by construction; the recall gate
  is a sanity check, not the proof. **This is the shippable real-latency search win this round.**
- **Files:** `crw-server/src/routes/search.rs` (fan-out ~1011-1143; page-2 ~264-302; multi-round
  ~449-543), `crw-search/src/client.rs`.
- **Risk:** 🟡 ordering bugs (gated by C1 source-set-equality test). **Flag:** `[search] pipeline_overlap`.
- **Effort:** M.

**Phase C2 — early-synthesis (start LLM before the slowest scrape) — DEFERRED, harness-gated.**
- **Blocked on a prerequisite:** building an answer-mode `gold-in-sources` fixture in search-bench
  (pinned queries, expected gold URLs/facts, baseline source-set capture, candidate comparison,
  answer/citation checks). Until that exists, early-synthesis CANNOT be proven quality-neutral and
  is **not implementation-ready** (Codex). It changes which sources feed the answer by definition.
- **Decision:** do NOT attempt C2 this round unless C1 + Phase 0 show the slowest-straggler wait is
  a large p90 share AND the team funds the gold harness first. Default off, deferred.
- **Risk:** 🔴 answer-quality regression if shipped without the gold harness → hard-gated.
- **Effort:** L (+ harness build). **Flag:** `[search] early_synthesis = false`.

### Phase D — Liveness SSE for answer-mode search (ack + heartbeat + final result)  *(saas; PERCEIVED only)*
- **🔴 Honest reframe (Codex finding — corrects a v2 overclaim):** `crwFetch` is **buffered** — it
  reads the full `res.text()` before returning (`crw-client.ts:33`) and the engine returns one
  `Json(resp)`. With one blocking buffered call, the SaaS **cannot** emit real `sources`/`answer`
  progress before the engine finishes. So Phase D does NOT stream sources/answer and gives **no
  TTFB win on the answer itself.** What it CAN do (and all it claims): send an immediate
  `event: searching` + headers at t=0 and a 15s heartbeat, keeping the connection **live** instead
  of a 25-40s frozen/blocked HTTP wait that proxies may kill and users read as "dead." The full
  result still arrives at the end in `event: done`. **This is a connection-liveness/UX win, not
  progressive streaming and not a latency cut.**
- **Real progressive streaming is OUT OF SCOPE this round** — it requires the engine to stream
  (axum `Sse`/chunked) AND `crwFetch` to stop buffering AND billing redesigned around streamed
  usage. Tracked as a future "Phase D2 (engine streaming)"; not attempted here.
- **Files:** `src/app/api/v1/search/route.ts` (engine call ~421; credit 274-757; partial-scrape
  refund **~503-521** and **~890-931**; 5-branch dispatch 537-731; `_meta` ~746-757),
  `src/lib/crw-client.ts`, `next.config.ts`, small `src/lib/sse.ts`.
- **Steps (ordering corrected — Codex/saas 🔴: preflight BEFORE opening SSE):**
  1. **Opt-in via ONE surface: `Accept: text/event-stream`** (drop `?stream=true`/`body.stream` →
     zero schema change). Default returns existing `NextResponse.json(...)` byte-for-byte.
  2. **PREFLIGHT as normal HTTP, BEFORE returning any SSE response:** auth → body parse/validate →
     rate-limit → **quota reserve** (`route.ts:274+`). All of these can still fail as **normal HTTP
     status + JSON** (401/400/429/credit-denied) — because no SSE/200 has been sent yet. This is the
     fix for "opening SSE before reserve makes error paths un-representable." `reserve` happening
     here (as it does today) means a credit-denied request never enters the stream path.
  3. **THEN return the SSE Response (HTTP 200)** and, inside the stream, emit `event: searching`
     immediately (first event is right after validation/reserve — a liveness win, **not literally
     t=0 before reserve**) + a **15s `: keepalive` heartbeat** while awaiting `crwFetch`.
  4. **Settle credits before `done`, with explicit abort wiring (must be BUILT — it does not exist
     today):** wire `request.signal` AND `ReadableStream.cancel` so a client disconnect aborts the
     `crwFetch` call (add optional `signal` param to `crw-client.ts`) and runs `refundAllReserves`.
     **Use a single settlement state-machine `unsettled → settling → settled`** (Codex): the FIRST
     caller (normal post-engine dispatch OR abort handler) transitions `unsettled→settling`
     **before** starting any async commit/refund work, wins, detaches the abort listener, clears the
     heartbeat, and performs its branch; any later caller sees non-`unsettled` and no-ops. Setting
     the flag only AFTER the async work completes would let an abort interleave at the first `await`
     inside the dispatch and double-refund. Confirm `refundCredits` is idempotent on `requestId` as
     defence-in-depth, else the state-machine is the sole guard.
  5. **On normal engine return:** transition `unsettled→settling` (step 4), run partial-scrape
     refund (`503-521`/`890-931`) AND the full 5-branch commit/refund dispatch (`537-731`) AND build
     `respondingBody` → mark `settled` → THEN `enqueue(done)`. **Rule: commit/refund completes before
     `done` is enqueued.** Engine 5xx → emit `event: error` + refund (also pre-`done`, same
     settlement state-machine).
  6. Headers: `Content-Type: text/event-stream`, `Cache-Control: no-cache, no-transform`,
     **`X-Accel-Buffering: no`** (nginx self-hosted `output:"standalone"` — else nginx buffers and
     the ack/heartbeat never reach the client), `Connection: keep-alive`. Add
     `export const dynamic="force-dynamic"` and `export const runtime="nodejs"`.
  7. `ReadableStream.start()` checks `request.signal.aborted` first → `controller.close()` if already
     aborted (no heartbeat interval leaking into a dead connection); `clearInterval` on cancel/close.
  8. `event: done` payload = **public** shape only (`{success,data}`); **strip `_meta`**.
- **Quality-safety:** the answer is identical and complete; only the wait is kept live. Non-streaming
  clients untouched.
- **Risk:** 🟡 proxy buffering (mitigated by X-Accel-Buffering + heartbeat); 🔵 modest value
  (liveness, not speed) — ship only if the frozen-wait UX is worth the code. **Opt-in only.**
- **Effort:** M.

### Phase E — Skip HTTP leg for preference-latched JS hosts  *(opencore; DEADLINE-SAFE rework)*
- **Status:** heavily reworked after v1 review found the fallback architecturally broken and the
  quality guarantee unprovable. **Kept only if Phase 0 shows latched hosts are a non-trivial share
  of p90 traffic** — on mostly-unique traffic (the same premise that killed caching) hosts rarely
  latch, so this may benchmark as a no-op (Codex/pragmatist). Phase 0 must report **promotion-hit
  rate**; if <~10%, drop this phase.
- **Files:** `lib.rs` (auto `fetch` 831-970; `fetch_with_js` 979; promotion 1048-1062; gate
  1164-1171; thin/fallback 1400-1574), `preference.rs`, `crw-core/src/config.rs`.
- **Reworked design (addresses 🔴 fallback + 🔴 budget + 🔴 PDF + 🔴 relative-gate):**
  1. **Budget-safe:** only skip HTTP when the remaining deadline can absorb a Chrome attempt AND a
     full-ladder fallback. Reserve a concrete `SKIP_HTTP_FALLBACK_BUDGET_MS` **calibrated from Phase
     0** (≈ p50 full-ladder time on the stealth rig, expected ~8s — NOT a token 1s that always
     passes and protects nothing). If Chrome-first + that reserve can't both fit the deadline, do
     NOT skip — run the normal ladder. Prevents "Chrome burns the deadline, fallback fails where
     baseline would have succeeded."
  2. **Real fallback path:** implement explicit re-entry — gate failure calls back into `fetch`
     with a `suppress_skip` flag that forces the normal HTTP-first ladder. (`fetch_with_js` cannot
     re-enter HTTP today; this re-entry must be built, not assumed.)
  3. **Relative gate, not absolute floor:** accept the Chrome result only if its text length is
     `>= 200` (new `SKIP_HTTP_MIN_TEXT_LEN`, NOT `MIN_RENDERED_TEXT_LEN=50`) AND passes all existing
     completeness checks. Because we can't compare against an HTTP body we didn't fetch, the gate is
     conservative; if it fails, the fallback fetches HTTP and the normal comparison/stitch applies.
  4. **PDF/non-HTML:** a latch is per-host but URLs vary. Do a cheap **HEAD (or 1-byte range) probe**
     for content-type before skipping; non-HTML → normal path. (Accept the small probe cost; it's
     far less than the HTTP body fetch this phase skips.)
  5. **De-latch oscillation:** document that `record_success` clears the latch (`preference.rs:103`);
     report latch oscillation in telemetry so it's not mistaken for a bug.
- **Quality-safety (honest bound — not overclaimed):** budget-reserved fallback + content-type probe
  guarantee we **never fail where baseline would have succeeded**. The 200-char gate is an absolute
  floor, NOT a comparison against the HTTP body we didn't fetch — so it CANNOT prove parity with
  what HTTP would have returned (a Chrome result of 400 chars ships even if HTTP would have given
  3000). The justification is the latch semantics: a host only latches after 3 Lightpanda failures
  in 15 min, i.e. HTTP was already established as unusable for it. **This makes Phase E a GATED
  EXPERIMENT, not a guaranteed quality-neutral lever** (Codex/quality consensus): ship only if the
  per-URL paired bench (candidate vs baseline) shows no markdown_len regression on latched hosts,
  default off.
- **Risk:** 🟡 unprovable strict parity (bounded by latch semantics + bench gate); 🟡 complexity;
  🟡 low p90 impact on unique traffic (gated by Phase 0 hit-rate). **Flag:**
  `[renderer] skip_http_for_latched_hosts = false`.
- **Effort:** L (was under-estimated as M in v1).

### Phase F — (Spike-gated, LAST, may be killed) Parallelize the Fetch intercept pump  *(opencore)*
- **Status: demoted from #1 to last.** The pump's own comment (`cdp.rs:771-775`) says the
  per-request roundtrip is **sub-millisecond on a local socket** and Chrome queues paused requests
  internally — i.e. this is a p50 micro-opt that overlaps render, unlikely to move a 15.4s p90.
  **Do NOT implement until Phase 0 telemetry proves `paused_request_count × pump_RTT` is a
  meaningful share of p90** (e.g. >300ms). If it isn't, this phase does not exist (YAGNI).
- **If proven worthwhile, implementation constraints (🔴 from CDP reviewer):**
  - Use `Arc<CdpConnection>` (cheap — all inner state already `Arc`-wrapped per `cdp_conn.rs:95`).
  - Use a **bounded `JoinSet`** (not raw `spawn`/`buffer_unordered`); **`abort_all` + drain the set
    BEFORE sending `Fetch.disable`** — otherwise detached tasks race cleanup and send a second
    response to an already-handled `requestId` (CDP `-32602`, can stall correlated requests).
  - Concurrency cap is bench-tuned (start 8); raise `EVENT_CHANNEL_CAPACITY` (currently 1024) or log
    on `RecvError::Lagged` — a dropped `Fetch.requestPaused` stalls that sub-resource.
  - Downgrade the "byte-identical" claim to "structurally-equivalent, proven by the recall gate"
    (concurrent continue/fail ordering can in rare cases affect script-dependency execution).
- **Risk:** 🟡 concurrency correctness; 🔴 cleanup race (mitigated above). **Flag:**
  `[renderer] intercept_pump_concurrency = 1` (default 1 = today).
- **Effort:** M+/L. **Order: dead-last, behind a measurement spike.**

> **Dropped from v1:** Phase 4 (lifecycle networkIdle — saves only poll slack, semantics may not
> match the manual tracker, undocumented on Lightpanda) and Phase 5 (blocklist/stylesheet tuning —
> content-affecting by nature, esp. analytics-as-gate hosts like `scorecardresearch.com`). Both are
> demoted to **separate future experiments, default off**, not part of this quality-neutral round.

---

## Verification (Phase V) — reworked for statistical validity

**Rig of record:** stealth rig, `crw-opencore-p90/bench/diagnose.py` → `p90_summary.py`, N=100,
conc 3, t/o 45s, **server restarted between arms** (drops leaked Chrome targets + breaker +
in-memory preference state — confirm restart clears the preference latch, Phase E).
**⚠️ Phase E exception (quality reviewer):** the restart-between-arms rule clears the in-memory
preference latch, so an E bench with restarts would have ZERO latched hosts and measure nothing.
For Phase E specifically, either run baseline+candidate arms WITHOUT a restart between them, or
pre-warm the latch (replay the host set once) before the candidate arm.

**🔴 Noise band first (was missing):** run the baseline arm **3 times** (or N≥300) to establish the
p90/p99 noise band: `noise = max(pairwise |p90| deltas)` across the 3 runs (a single pair's delta is
itself noisy). A candidate counts as a win only if `baseline_p90 − candidate_p90 > noise`. For p90
at N=100 a single order-statistic swap can be ±1-3s, so treat any sub-noise delta as null.

**Per-phase protocol (baseline vs candidate, same pinned URL set):**
```bash
cd /Users/us/coding/crw/crw-opencore-p90
# Pin the dataset: save the first-100 URL set as a fixture and reuse via --urls-file so the
# `labeled` denominator can't drift between arms. Assert both JSONL files share the same id set.
CRW_API_URL=http://localhost:3000 bench/.venv/bin/python bench/diagnose.py \
  --max-urls 100 --concurrency 3 --timeout 45 --output bench/server-runs/<phase>-<arm>.jsonl
bench/.venv/bin/python bench/p90_summary.py bench/server-runs/<phase>-<arm>.jsonl <arm-label>
```

**🔴 Runnable quality gate (was prose-only):** write `bench/quality_gate.py` that joins
baseline+candidate JSONL on `id` and FAILS if:
- **(first, as a hard error)** `set(baseline_ids) != set(candidate_ids)` — never compare different
  populations;
- **(never-new-failure gate, Codex)** any URL where baseline succeeded now hard-errors/times-out.
  **Retry procedure for the escape clause:** re-run `diagnose.py` for that single URL in isolation
  (N=1, no concurrency) against BOTH arms in the same session — if it also fails on baseline,
  exclude it (anti-bot/rate-limit noise from the bench itself); if it passes baseline but fails
  candidate, the gate FAILS. No ad-hoc judgement.
- aggregate `truth_recall` drops > noise band (use **±0.25pp for quality-affecting phases C2/E/F**,
  ±0.5pp for capacity/scheduling phases A/B/C1);
- `median(candidate_markdown_len)/median(baseline_markdown_len) < 0.95`;
- any single URL's `markdown_len` drops > 30% or gains a `lie_text` leak or a worse status.
- **Prerequisite (bench reviewer):** before Phase A is benchmarked, smoke-test `quality_gate.py` on
  (a) two identical JSONL files → must PASS, (b) a fabricated thinned JSONL → must FAIL on
  markdown_len. A buggy gate silently passes regressions.

**Search side — Phase C1 (scheduling overlap, identical source set):** the gate is
**source-set equality** — assert the candidate's final source set == baseline's for each query (run
SearXNG via a fixed/mocked response in the test so retrieval churn doesn't cause spurious diffs,
per bench reviewer). C1 is quality-neutral by construction; this is the proof.
**Phase C2 (early-synthesis):** BLOCKED until the answer-mode `gold-in-sources` fixture exists in
search-bench (does not today — `analyze.py` only does domain overlap). Not gated, not shipped.

**Phase D verification (liveness, NOT progressive streaming — corrected):**
- structural: events are `searching` (immediate) → `: keepalive` heartbeats → `done` (full result).
  There are **no** `sources`/`answer` progress events (the engine is buffered) — assert exactly this
  shape; `done` payload schema-valid, `_meta` absent.
- equivalence: `done.data` equals the non-streaming JSON `data` (sources + everything) for the same
  query with a **mocked engine response** (LLM is nondeterministic live — do NOT assert on live
  answer text); `llmUsage`/credit charge identical between paths. **Mock at least two shapes:**
  (a) clean full-enrichment success, (b) partial enrichment that triggers the refund at
  `503-521`/`890-931` — to prove the post-refund `data` is what `done` emits.
- ordering: with a spy on the credit functions, assert **commit/refund completes before `done` is
  enqueued** (the ack-vs-billing invariant — distinct from the disconnect/server-kill tests).
- the (only) win: **headers + `searching` arrive at t≈0 and the connection stays live** through a
  >25s engine call (assert heartbeats received). NO answer-TTFB claim — the answer still lands at
  the end.
- billing: client-disconnect test — disconnect mid-wait, assert refund fired via `request.signal`
  and no double-charge / no free answer; plus a server-kill-between-engine-return-and-`done` test
  asserting the charge is retained and the connection closes (not hangs).

**Validity caveats (acknowledge, don't ignore):** local bench p90 is **directional only** (local
CPU hosts client+server+Chrome+LP together; network RTT differs from the Hetzner prod box). Before
any public latency claim or prod rollout of Phase **A/C/E**, run a **prod shadow bench** (crw on the
prod VPS, same pinned URL set, flag off→on) — Phase C's fan-out timing is even more sensitive to
real network RTT than A/E. Prod-shadow is a **BLOCKING exit gate for Phase A** (a local "size 6 wins"
can OOM/regress on the 8GB Hetzner box). The N=1000 stealth prod run in `P90_RESULTS.md` is the gold
standard for replication.

**Unit/integration:** `cargo test -p crw-renderer` (Phases A/B/E/F); `bun test` SaaS route (D).

## Open questions

1. Phase 0: **pin the p90-budget-share "worth it" threshold to 300ms BEFORE running Phase 0**
   (pre-committing avoids moving the goalpost post-hoc to justify a phase). AND: is the stealth-rig
   tail queue-wait or in-render? (decides whether Phase A has any lever)
2. Phase A: which pool knob limits the stealth rig/prod (`pool_size=4` vs context-pool
   `max(2,n/2)`)? + RAM/OOM ceiling on 8GB with stealth Chrome.
3. Phase C2 (early-synthesis): deferred — needs an answer-mode `gold-in-sources` harness built
   first. Worth funding only if Phase 0 shows straggler-wait is a large search-p90 share.
4. Phase E: is the HEAD/range content-type probe cheap enough to keep the phase net-positive?

## Order of execution & risk table

| Phase | Repo | Targets | Effort | Risk | Order |
|---|---|---|---|---|---|
| 0 — instrument & measure | opencore | p90 breakdown + queue-vs-render + hit-rate | S–M | 🔵 | **1 (prereq)** |
| A — pool size + Chrome flags | opencore | render-capacity tail (if queue-wait) | M | 🟡 OOM | 2 |
| C1 — pipeline scheduling overlap | opencore | answer-mode real latency, same source set | M | 🟡 ordering | 2–3 |
| B — DNS cache + proxy-client audit | opencore | DNS resolution (likely marginal) | S | 🔵 no-op risk | 3 (parallel) |
| D — SaaS liveness SSE (perceived) | saas | frozen-wait UX, NOT speed | M | 🟡 proxy | 3 (parallel, separate repo) |
| E — skip-HTTP for latched hosts | opencore | p50 on latched hosts | L | 🟡 unprovable parity / low-impact | 4 (gated by Ph0 hit-rate) |
| F — parallel intercept pump | opencore | pre-render overhead | M+/L | 🟡 concurrency | 5 (spike-gated; may be killed) |
| C2 — early-synthesis | opencore | answer-mode straggler cut | L+harness | 🔴 recall | DEFERRED (needs gold harness) |

Rollback: every phase behind a default-off flag (or opt-in request) → rollback = flip flag / revert
one commit; no schema or data migration.

## Iteration log
- **Iteration 1**: addressed 8 critical / ~11 warnings from 5 reviewers + Codex. Major restructure:
  added **Phase 0** (measure-first) and a **real-latency block** (pool sizing, conn reuse/DNS,
  concurrent fan-out) that targets the actual Chrome-render p90 tail; **demoted Phase 1→F**
  (spike-gated, code comment says sub-ms); **reworked Phase 2→E** (deadline-safe fallback, real
  re-entry path, relative gate, content-type probe); **restructured Phase 3→D** (commit-before-
  stream billing fix, Accept-only opt-in, mandatory heartbeat, X-Accel-Buffering, signal wiring,
  _meta strip, TTFB/sources-set verification instead of byte-equality); **dropped Phases 4/5** to
  future experiments; **reworked Verification** (p90 noise band, runnable markdown_len gate script,
  dataset pinning, tighter gate for quality-affecting phases, localhost-directional caveat +
  prod-shadow requirement); documented the **pre-existing partial-with-refund** search contract as
  out of scope. Rejected nothing outright; folded all findings in.
- **Iteration 2**: 5 reviewers reached [CONSENSUS] (residual 🟡 only); Codex raised 2 new 🔴. Fixed:
  **Phase D honestly reframed** — buffered `crwFetch` cannot stream sources/answer, so D is now
  *liveness only* (immediate ack + heartbeat + final result; no answer-TTFB claim); real progressive
  streaming split out as future D2 (engine streaming). **Phase C split** into C1 (scheduling overlap,
  identical final source set → quality-neutral by construction, shippable) and C2 (early-synthesis,
  DEFERRED behind building an answer-mode gold-in-sources harness that doesn't exist today). **Phase
  A** pool-config disambiguated (`pool_size=4` vs context-pool `max(2,n/2)`) + queue-wait-vs-in-render
  made the A go/no-go discriminator + prod-shadow a blocking exit gate + honest negative-result path.
  **Phase B rescoped** (connection reuse mostly already exists; real gap is DNS cache; likely
  marginal). **Phase E** quality claim de-overclaimed to a gated experiment + `SKIP_HTTP_FALLBACK_
  BUDGET_MS` calibrated from Phase 0. **Verification** hardened: 3-run noise band (max pairwise),
  id-set equality as hard error, never-new-failure gate, quality_gate.py smoke-test prerequisite,
  C1 source-set-equality (mocked SearXNG), D liveness tests + server-kill test, Phase C added to
  prod-shadow. Also tightened Phase D billing to include the partial-scrape refund (503-521/890-931)
  pre-stream and the `ReadableStream.start` aborted-check. Rejected nothing; all findings folded in.
- **Iteration 3**: quality/pragmatist/bench reached [CONSENSUS]; saas + Codex raised 1 shared 🔴 on
  Phase D ordering. Fixed: **Phase D re-sequenced** — PREFLIGHT (auth/body/rate-limit/quota-reserve)
  runs as normal HTTP BEFORE any SSE/200 is sent (so credit-denied/4xx/429 stay representable as
  JSON status), THEN the SSE Response opens with immediate `searching` + heartbeat, THEN
  commit/refund completes before `done`. **Built the abort-refund wiring** that v3 wrongly called
  "existing": `request.signal` + `ReadableStream.cancel` → `refundAllReserves`, guarded by a
  `creditSettled` flag (kills the double-refund race) + idempotency note. Softened the "t=0" claim
  to "first event after validation/reserve." Verification: added the **ack-vs-billing ordering test**
  (spy asserts commit before `done`), the **mocked-engine partial-refund shape**, a defined
  **never-new-failure retry procedure** (isolated N=1 re-run both arms), the **C1 input-set-final
  precondition**, the **Phase E no-restart bench exception**, and **pinned the Phase-0 300ms
  threshold** pre-commit. Rejected nothing.
- **Iteration 4** — **CONSENSUS**: all 5 reviewers + Codex at consensus. saas reviewer traced every
  billing scenario (A–E) clean; Codex confirmed preflight-before-SSE resolves the 4xx/429 JSON-status
  blocker. Final fold-in: the credit settlement is now a single **`unsettled → settling → settled`
  state-machine** (transition before async work, first caller wins, detach listener + clear
  heartbeat) — closes Codex's last 🟡 (abort interleaving at the first `await` of the dispatch).
  No 🔴/🟡 remain.
