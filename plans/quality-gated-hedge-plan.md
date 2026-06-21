# Quality-Gated Conditional Hedge — Implementation Plan (v2)

> Worktree `/Users/us/coding/crw/crw-opencore-latency` (branch `perf/latency-qn`). Rust engine only.
> v2 = major restructure after iteration-1 review (5 agents): **priority inverted** — the
> success-raiser (auto-egress) and telemetry ship first; the L-effort race core is demoted to a
> bench-gated, revertible experiment, because the measured tail is in-render Chrome the race can't cut.

## Context

fastCRW escalates **serially** (HTTP → LightPanda → Chrome), each tier's failure adding its full
latency → 15.4s p90 / 37s p99 tail. We want to cut that tail WITHOUT dropping scrape-success.

### THE HARD RED LINE
**scrape-success must never drop below the serial baseline (which must itself be shown ≥ Firecrawl
on the same harness).** Every change below is gated on this.

### Measured reality that reshaped this plan (PHASE-RESULTS.md Phase 0, P90_RESULTS.md)
- The p90/p99 tail is **100% browser render** (Chrome-dominant); HTTP's tail share is **0**. The tail
  URLs are anti-bot pages with a **~10s Chrome render floor** (P90_RESULTS.md:142).
- Therefore a race only saves the **serial LP leg (~2.5s timeout)** on pages that escalate to Chrome
  anyway — it CANNOT cut Chrome's in-render time. Realistic race p90 win ≈ A4's −18% (15.4→~12.7s)
  but A4 paid −4.5pp recall; a quality-gated race aims for that win at lower recall cost — **single
  digit %, competing with the already-shipped pool=8 (−34% p90 under load).**
- **Consequence (iteration-1 consensus): the race core is the WEAKEST ROI.** The highest-value,
  lowest-risk piece is **auto-egress escalation** (only *adds* a recovery arm on already-failing
  pages → can only raise success). So we **ship auto-egress first, standalone**, and treat the race
  as a bench-gated experiment we are prepared to revert (like the last hedge, P90_RESULTS.md:143).

### The 89 vs 89.7 premise — right-sized
Serial baseline success 89% vs Firecrawl 89.7% is **one URL at N=100, inside the run-to-run noise
band** (P90_RESULTS shows 87/88/89% bouncing). Do NOT treat 0.7pp as a hard gap to chase. Phase 0
establishes the noise band first; act on the FC delta only if it's *outside* the band. Auto-egress is
desirable headroom regardless, not "mandatory to close 0.7pp."

### Verified facts from research (cite)
- **Race mechanics** *(race-mechanics agent)* — Spider `website.rs:597-661` + `parallel_backends.rs:553-696`:
  `JoinSet` + `select!{biased; join_next; sleep_until(deadline)}`, grace deadline opened **lazily**
  only once the first contender arrives (`:664`), fast-accept on first result ≥ threshold (`:651`),
  `abort_all()`+drop on accept (drop alone aborts; explicit call = eager signal, `:686`).
- **Cancellation safety** *(race-mechanics)* — fastCRW ALREADY has the Spider `TabCloseGuard` pattern:
  `PoolGuard::Drop` reaper + terminator `AtomicBool` (browser_pool.rs:856-941, 146-155) closes
  page→context→ws on drop. Race-losers drop their `PoolGuard` → no new leak machinery.
- **Quality scoring** *(scoring agent)* — Spider `html_quality_score` (parallel_backends.rs:210-281):
  status(30/20/5/0)+length(5/10/10)+structure(`<body`:15, non-empty:10)+antibot(None:**+20**); asset
  branch awards 25. fastCRW predicates exist in detector.rs/lib.rs. **CRITICAL:** the body-text axis
  MUST use `extract_body_text_len` (detector.rs, strips `<script>`), NOT `html_body_text_len`
  (lib.rs:1643, does NOT strip scripts → SPA shells with big inline JS inflate the score).
- **Resource bounds** *(bounding agent)* — failure-mode→knob map: pool exhaustion (browser_pool.rs
  semaphore :246,314); host-permit double-acquire/deadlock (host_limiter.rs :83-92, `per_host_cap_serializes`
  test :166) — **race arms must NOT call `fetch_inner` (re-acquires host permit → deadlock); hold ONE
  host permit outside, arms call `renderer.fetch()` directly**; breaker miscount-on-cancel (use
  `cancel_probe` :424 / `ProbeGuard` :766 / `record_scoped_outcome` :640, never `record_result(false)`);
  false promotion from raced loss (preference.rs `counts_for_promotion` :12). Spider's
  `max_concurrent_sessions` default is a **fixed 8**, NOT pool-size (configuration.rs:304).
- **Success floor** *(success-safety agent)* — today's `thin_result` stitch keeps **max-by-`html.len()`**
  across tiers, errors only if every tier errored (lib.rs:1448-1477, 1590-1626). Auto-egress fires on
  the **hard-block subset** (403/429/503/401/520-530 + CF/bot-wall/vendor markers), NOT 404/410/412/451/500.
- **Bench** *(bench agent)* — success per URL = `scrape_ok` (`body.success AND non-empty markdown`,
  diagnose.py:81-84); join key `id`; **`diagnose_3way.py` does NOT read the fixture** (loads HF directly,
  crw on :3030, recall metric md+strip_links ≠ diagnose's md+plainText) → use **`diagnose.py --api-url`
  as the single canonical red-line tool**; `quality_gate.py` doesn't exist; p90 at N=100 = one order
  statistic → raise to **N=200-300**, ≥2-3 runs, report band as min/max; stealth arm mandatory.

## What we are NOT doing (scope guard)
- ❌ Blind hedge (fired browser on every slow page; consume 11/63). Race fires browser ONLY on
  low-score results AND consults `preferred()`; consume-ratio is instrumented (Phase 1).
- ❌ Any success/recall regression — best-result-wins (max-`html.len()` over ALL arms incl. the
  auto-egress arm) preserves today's floor; flag-off path byte-identical.
- ❌ Fast-mode / timer-trim (success loss).
- ❌ Speculative proxy spend — auto-egress fires only AFTER an observed hard-block, one attempt.
- ❌ Search/answer path (scrape renderer only).
- ❌ `max_hedge_sessions = pool_size` (would let the hedge starve the whole pool — re-creates the
  flood). Use a fixed small cap (8) or `pool/2`.

## Phases (priority-ordered: prove the harness, raise success, then the gated race experiment)

### Phase 0 — Baseline, canonical harness, red-line gate (PREREQUISITE)
- **Rebuild the local rig** (torn down for disk): `cargo build --release -p crw-server --features cdp`;
  docker lightpanda→9222 + chromedp/headless-shell(prod digest)→9223; `config.local-bench.toml`. Stealth arm.
- **Canonical red-line tool = `diagnose.py --api-url`** (it takes `--urls-file` + `--api-url`; the only
  one that runs on the pinned fixture with one metric). Demote `diagnose_3way.py` to the N=1000
  public-claim run only. Pin crw port consistently (`:3000`).
- **Write `bench/quality_gate.py`** — join baseline+candidate JSONL on `id`; **HARD-FAIL if** (a) any
  `id` with baseline `scrape_ok==True` is candidate `scrape_ok==False` (success red line), OR (b) any
  such id's `markdown_len` drops > **30%** (the completeness red line — `scrape_ok` parity alone lets a
  thin-but-non-empty result pass; this gate is the single most important hardening). Report recall
  deltas. Smoke-test on identical files (PASS) + fabricated regressed file (FAIL).
- **Serial baseline, N=200-300, ≥3 runs** → p50/p90/p99 **noise band (min/max)** + `scrape_ok` count +
  recall, stealth arm.
- **FC same-harness OR honest fallback:** if an FC container is available, run it through the SAME
  `diagnose.py --api-url` on the SAME fixture for an apples-to-apples `scrape_ok` count. If not
  provisioned (P90_RESULTS.md:178), the red line reduces to **"never below our serial baseline"** (which
  IS runnable) and the "≥ FC" claim is deferred to prod-shadow — stated explicitly, not blocking.
- **Effort:** M. **Risk:** 🔵.

### Phase 1 — Hedge/decision telemetry (BEFORE any engine change)
- **Why first:** the last hedge died on the consume ratio (63 fired / 11 consumed) and was invisible.
  Any later experiment must be observable from the first request.
- **Files:** extend the shipped `latency_breakdown` target (PHASE-RESULTS Phase 0) in lib.rs.
- **Steps:** emit per-request: accepted tier, `fast_accept` hit, `escalation_fired`, `hedge_fired`,
  `hedge_consumed` (which arm's result actually won), `auto_egress_fired`/`consumed`, per-tier score.
  **🔴 Per-attempt timing (Codex P90 hypothesis):** also log per-tier **start/end timestamps, outcome,
  score, and escalation reason** — total-ms + accepted-tier alone CANNOT prove whether the tail is
  stacked failed-tier time (which the race cuts) vs the final Chrome navigation itself (which it
  doesn't). This per-attempt breakdown must exist BEFORE Phase 4 so the race's value is provable, not
  assumed. Behind the existing `latency_breakdown` flag (off in prod, on for bench).
- **Effort:** S. **Risk:** 🔵.

### Phase 1.5 — Read-the-telemetry KILL-GATE for the race (before building Phase 4)  *(decision)*
- **🟡 (bench review — pure win):** Phase 1's per-attempt start/end timing, collected on the rebuilt
  rig in Phase 0/1, can largely answer "is the tail **stacked failed-tier time** (which the race
  cuts) or **the final Chrome navigation itself** (which it does NOT)?" PHASE-RESULTS Phase 0 already
  strongly suggests the latter (Chrome p50 9.2s in-render). **Decision gate: if the per-attempt
  timing confirms the tail is dominated by the final Chrome nav rather than stacked failed-tier time,
  DO NOT build Phase 4** (YAGNI — the same discipline that correctly killed Phase F). This converts
  "build the L-effort race then revert" into "never build it." Phases 2 (auto-egress) and the rest
  proceed regardless; only the Phase 4 race is gated here.
- **Effort:** S (analysis). **Risk:** 🔵.

### Phase 2 — Auto-egress escalation on hard-block (the success-raiser, SHIP FIRST)  *(opencore)*
- **Highest value / lowest risk: only ADDS a recovery arm on already-failing pages → success can only
  rise; clean pages pay zero.** No dependency on the race core.
- **Files:** lib.rs (status/marker classify :933-936/:1193, proxy machinery `REQUEST_PROXY` :1082,
  chrome_proxy tier :1099), detector.rs markers, config.rs flag.
- **Steps:** when the basic result is a **hard block** — status ∈ {401,403,429,503,520-530} OR
  `cloudflare_challenge`/`generic_bot_wall`/`vendor_block`/`cloudflare_mitigated` — fire **one**
  alternate-egress retry (BYO/residential proxy + stealth tier), **after** the basic result is in (not
  speculative), **gated** by `deadline.remaining() ≥ tier_timeout` AND host breaker closed. Exclude
  404/410/412/451/500.
- **🔴 Success-safety (from review):** the auto-egress result MUST go through **best-result-wins vs the
  basic result** (max-`html.len()`, content-gated) — if the proxy arm returns `Ok(empty)`, KEEP the
  basic thin result. Never let an empty retry replace usable content.
- **Risk:** 🟡 residential cost/latency on doomed pages → one attempt, deadline+breaker gated.
  **Effort:** M. **Flag:** `[renderer] auto_egress_escalation=false` + env `CRW_RENDERER__AUTO_EGRESS_ESCALATION`.

### Phase 3 — `html_quality_score(0-100)` (pure fn, benched standalone before the race)  *(opencore)*
- **Files:** new `crates/crw-renderer/src/quality.rs`; expose `extract_body_text_len` from detector.rs
  as `pub(crate) fn body_visible_text_len`.
- **🔵 Return reason flags, not just a number (Codex):** the scorer returns a struct
  `{ score, fast_acceptable, should_try_baseline_js, hard_block_for_egress, permanent_for_egress }` so
  escalation *policy* isn't encoded in one numeric threshold — threshold tuning in Phase 7 then can't
  silently change escalation/egress semantics. The race/egress phases branch on the flags; `score` is
  only the race tie-break.
- **Steps — true 0-100 (anti-bot axis restored per review #1):**
  - Status 0-25: 200→25; 2xx/304→18; 3xx→5; `is_status_blocked` (sync to include **520-530** per
    review #4)→0.
  - Body text 0-35 via **`extract_body_text_len`** (strips scripts — review #2 critical): <50→0;
    50-199→10; 200-999→22; ≥1000→35.
  - Structure 0-15: real non-empty `<body>`.
  - **Anti-bot 0-25** (positive axis): no block/challenge signal → +25.
  - **Hard caps (safety):** vendor-block/bot-wall/cloudflare/antibot-blocked → clamp ≤10;
    failed-render/loading-placeholder/needs-js → clamp ≤30.
  - **Asset/binary branch:** only when `len>0 && status.is_success()` (review #5) award status+structure,
    skip body-text axis; Phase 5 must skip racing binary.
- **`fast_accept_threshold = 70`** (clean page now 25+35+15+25=100). **Completeness-predicate bypass
  (review #3):** if status==200 AND `extract_body_text_len ≥ MIN_RENDERED_TEXT_LEN(50)` AND **NO cap
  predicate fires** → fast-accept regardless of score. **🔴 "no cap predicate fires" MUST include ALL
  SIX cap predicates (both the ≤10 block set AND the ≤30 escalate set):** `vendor_block.is_none()` &&
  `!generic_bot_wall` && `!cloudflare_challenge` && `!loading_placeholder` && `!needs_js_rendering` &&
  `failed_render.is_none()`. Otherwise a thin SPA shell (text=60 + `id="__next"`) would satisfy
  status==200+text≥50 and false-accept (scoring-review 🔴). This makes the bypass exactly mirror the
  existing completeness predicate (lib.rs:1212). `thin_floor ≈ 30` → existing `thin_result` last-resort.
- **🟡 Two implementation contracts (scoring review):** (1) the `body_visible_text_len(html: &str)`
  wrapper MUST `to_lowercase()` before delegating to `extract_body_text_len` (which expects pre-lowered
  input + does a `contains("<body")` check — uppercased `<BODY>` else triggers the 1000-char fallback
  → false +35); (2) the **anti-bot +25 axis and the ≤10 cap MUST use the SAME four predicates**
  (`vendor_block`/`generic_bot_wall`/`cloudflare_challenge`/`antibot_result`); (3) add 520-530 to
  `is_status_blocked` at **BOTH** `quality.rs` and the existing `lib.rs:1195` in the same commit (else
  the scorer's `hard_block_for_egress` diverges from the serial ladder's block classification).
- **`fast_acceptable` flag contract:** it must be set by *running the completeness predicate*, not by
  `score ≥ 70` alone; `hard_block_for_egress`/`permanent_for_egress` are populated independently of
  `fast_acceptable` (a 200-OK CF-challenge page is `fast_acceptable:false` + `hard_block_for_egress:true`).
- **Standalone validation (review, pragmatist):** replay the baseline run's recorded HTML through the
  scorer offline; confirm the score-at-70 partition matches today's boolean completeness gate
  (lib.rs:1212) on the corpus BEFORE Phase 4 wires it. If they disagree, fix the score first.
- **Risk:** 🟡 threshold tune (caps + bypass do the safety work). **Effort:** M.

### Phase 4 — Conditional-hedge race core (BENCH-GATED EXPERIMENT, revertible)  *(opencore)*
- **Status:** explicitly an experiment. The measured tail is in-render Chrome the race can't cut;
  expected win is the LP-serial-leg save (~2.5s) on the escalating subset. **Ship only if Phase 7 bench
  shows a p90 win beyond the noise band AND the consume ratio (Phase 1 telemetry) is healthy; otherwise
  revert** (like the last hedge).
- **Files:** lib.rs (refactor fetch path behind the flag; serial path preserved when off), browser_pool
  PoolGuard, quality.rs.
- **Steps:**
  1. Acquire **exactly one** host permit for the URL; **arms call `renderer.fetch()` directly, NOT
     `fetch_inner`** (review C1 — avoids host-permit re-acquire deadlock).
  2. Cheap tier (HTTP) → score → fast-accept if ≥ threshold (browser pool untouched).
  3. Else lazy-grace `JoinSet` race of next tier(s) (consult `preferred()` for order);
     `select!{biased; join_next; sleep_until(deadline)}` with the deadline **only polled once a
     contender exists** (review W1 — no `sleep_until(FAR_FUTURE)` timer churn).
  4. **best-result-wins** over ALL completed arms (max-`html.len()`, content-gated text_len≥50); error
     only if all arms errored.
  5. **🔴 Grace must NOT truncate the baseline (Codex):** the grace window governs only *when the next
     tier joins the race* — it must NEVER cause early give-up below the serial baseline. If no arm
     reaches `fast_accept_threshold` within grace, the tiers keep running to the **full baseline
     deadline** (same tiers serial would have tried), and we return best-result-wins at the deadline.
     Fast-accept is permitted ONLY for a result that satisfies the existing completeness predicate
     (lib.rs:1212); a low-quality arm never short-circuits the recovery the serial LP→Chrome path
     would have achieved. This is the core success-floor guarantee. **🔵 Two-phase loop (concurrency
     S-NEW-2):** grace expiry must ONLY stop *spawning new arms* — it must NOT break the `join_next`
     loop; keep joining running arms until the real `deadline`. Naive `sleep_until → break` would kill
     the in-flight Chrome arm. Phase: (1) within grace — fast-accept or spawn next; (2) after grace —
     join only, to deadline.
  6. **🔴 Proxy task-local propagation (Codex + concurrency W-NEW-2):** `REQUEST_PROXY`/country is a
     tokio **task-local**; `JoinSet::spawn`ed arms do NOT inherit it → each spawned arm MUST be
     wrapped in `REQUEST_PROXY.scope(..)` + country scope. **AND** the renderer-selection logic itself
     (the `proxy_active` check + `preferred()` + LP-drop-when-proxied at lib.rs:1082-1108) must run
     *inside* the proxy scope (or capture `proxy_active` outside and pass it in) — else the orchestrator
     picks the tier list seeing `proxy_active=false`, includes LP, and the arm later finds proxy active
     but LP can't proxy → the serial path's `no proxy-capable renderer` hard-error is silently skipped.
     Integration test: a BYOP-proxied request's raced arms actually egress through the proxy.
  7. On accept: `abort_all()`+drop JoinSet; losers drop `PoolGuard`. **Note (Codex):** `PoolGuard::Drop`
     is a **detached `tokio::spawn`** cleanup (browser_pool.rs:856), NOT a bounded reaper — mass loser
     aborts create bursts of detached cleanup tasks. Add a **cancellation stress test** around
     `abort_all()` asserting pool inflight/idle gauges return to baseline (no context leak, no permit
     leak); consider a bounded reaper channel if the burst proves harmful. **Guard the
     `acquire()`-mid-health-check ghost-slot leak (review C2)** — note (concurrency W-NEW-1) `acquire()`
     has TWO distinct cancel points needing DIFFERENT cleanup: a cancel during `health_check()` (slot
     popped from idle) must `mark_slot_dead_and_drop`; a cancel during `create_browser_context()` (conn
     exists, no guard yet) must `close_conn()`. One scopeguard can't cover both — handle each path.
- **Risk:** 🔴 concurrency — mitigated by reusing guards + Phase 5 bounds + Phase 6 accounting + the
  flag-off golden test. **Effort:** L. **Flag:** `[renderer] hedge_quality_gated=false` + env override.

### Phase 5 — Resource bounds  *(opencore, with Phase 4)*
- **`max_hedge_sessions`** = fixed **8** (or `pool/2`), NOT pool_size (review #2 critical — never let the
  hedge consume the whole pool / starve non-hedge requests). A `Semaphore` on top of the pool.
- **Lock order documented + enforced:** always `hedge_session_sem` BEFORE `pool.sem`; serial path never
  takes `hedge_session_sem` (review W4).
- **`max_hedge_bytes_in_flight`** (256MiB) caps HTML buffers — acknowledge it does NOT bound Chrome
  render RAM; the real RAM guard is pool size + session cap, validated against the **post-pool-8 RAM
  baseline** on the 8GB box (review #3), not a fresh box. **🟡 Explicit canary RAM watch (review):**
  the arithmetic peak to watch is **(pool=8 baseline contexts + up to `max_hedge_sessions`=8 racing
  contexts) × per-context RAM vs the chrome `mem_limit`=3g**; state this peak as the canary kill
  threshold, don't just say "watch RAM."
- Per-tier `connect_timeout` + `backend_timeout` (cancelled → `DeadlineClamped`/`RaceCancelled`, not a
  breaker failure).
- **🔴 Do NOT skip baseline escalation for 404/410/412/451/500 (Codex):** the current baseline
  *escalates* those statuses to JS and recovers content from them (lib.rs:921) — skipping that would
  violate the red line. These are excluded ONLY from the alternate-**EGRESS** arm (Phase 2, a
  different IP won't fix them), never from the normal renderer escalation/race. Skip *racing* only for
  binary/asset content-types (no HTML quality variance).
- **Effort:** M. **Risk:** 🟡 caps need bench validation.

### Phase 6 — Breaker & preference correctness under racing  *(opencore, with Phase 4)*
- Add a **`BreakerOutcome::RaceCancelled`** variant (`advances_window=false`, `is_failure=false`) +
  thread a cancellation token so a cancelled race-loser records NO failure (review W2 — else cancelled
  arms log spurious `TierTimeout`). **🔵 (concurrency S-NEW-1):** a cancelled arm should map
  `Err(CrwError::Cancelled)` → `BreakerOutcome::RaceCancelled` **directly**, bypassing
  `classify_outcome` (whose signature has no "race-cancelled" input) — cleaner than threading a new
  bool through it.
- Route race-losers through `cancel_probe`/`ProbeGuard` drop; content-quality losses via
  `record_scoped_outcome` (host tier only, never the global per-renderer window, review W3); call
  `record_failure` (promotion) ONLY on genuine low-quality, never on a mere raced loss.
- **Effort:** M. **Risk:** 🟡 accounting (covered by tests + consume-ratio telemetry).

### Phase 7 — Verification + rollout  *(opencore)*
- Per shipped phase: baseline (flags off) vs candidate (flags on), server restarted between arms,
  stealth arm, pinned fixture, N=200-300.
- **PASS GATE (all):** (1) `quality_gate.py` zero baseline-success→candidate-fail AND zero >30%
  markdown_len drops; (2) `scrape_ok` count ≥ baseline (≥ FC per Phase 0); (3) recall ≥ baseline −0.25pp;
  (4) p90 < baseline noise-band floor (else the phase is a no-op → not shipped / reverted).
- **Tuning:** sweep `fast_accept_threshold` (60/70/75) + `grace_ms` (300/500/800); pick max p90 cut
  subject to gates 1-3. Validate the **consume ratio** (Phase 1 telemetry) is healthy (low fired-but-
  not-consumed).
- **Rollout (review — missing in v1):** **env-var kill-switch** `CRW_RENDERER__HEDGE_QUALITY_GATED=false`
  (instant off, no redeploy). Sequence: **prod-shadow (metrics only) → canary (fraction) → full**,
  watching consume-ratio + p90 + success + **RAM on the already-pool-8'd 8GB box** at each step.
- **Effort:** M. **Risk:** 🔵.

## Open questions
1. Phase 3 weights/threshold — defaults pinned (70 / anti-bot +25 / caps 10&30 / completeness bypass);
   tune in Phase 7.
2. Phase 4: race LP+Chrome parallel vs LP-then-Chrome staggered? Staggered bounds pool load; decide via
   Phase 5 caps + bench.
3. Phase 2/Phase 0: is the residential/stealth tier wired locally for bench, or prod-shadow only?
4. Is the FC container provisioned for the same-harness run, or does the red line stay "≥ serial
   baseline" with FC deferred to prod-shadow?

## Order of execution & risk table
| Phase | Targets | Effort | Risk | Order |
|---|---|---|---|---|
| 0 — baseline + diagnose.py canonical + quality_gate.py (markdown_len gate) | premise + red line | M | 🔵 | 1 (prereq) |
| 1 — hedge/decision telemetry | observability (consume ratio) + per-attempt timing | S | 🔵 | 2 |
| 1.5 — read-telemetry KILL-GATE for race | don't build Phase 4 if tail is in-render | S | 🔵 | 2.5 (gates Ph4) |
| 2 — auto-egress escalation | **success raiser (highest ROI)** | M | 🟡 | 3 (ship first) |
| 3 — html_quality_score (extract_body_text_len, anti-bot axis, bypass) | the gate signal | M | 🟡 | 4 |
| 4 — conditional-hedge race core | tail cut (EXPERIMENT, revertible) | L | 🔴 | 5 (bench-gated) |
| 5 — resource bounds (fixed session cap, lock order) | safety on 8GB box | M | 🟡 | 5 (with 4) |
| 6 — breaker/preference correctness (RaceCancelled) | no spurious learning | M | 🟡 | 5 (with 4) |
| 7 — verification + shadow→canary→full rollout | the red line + ops | M | 🔵 | 6 (gates all) |

Rollback: every phase behind a default-off flag + env kill-switch; off = byte-identical serial ladder.

## Iteration log
- **Iteration 1** (5 agents): major restructure. **Inverted priority** — telemetry (Ph1) + auto-egress
  success-raiser (Ph2) ship before the L-effort race, now a bench-gated revertible **experiment** (Ph4),
  because the measured tail is in-render Chrome the race can't cut. Fixed: **score must use
  `extract_body_text_len`** not html_body_text_len (SPA-shell inflation); **restored the +25 anti-bot
  axis** (true 0-100); **completeness-predicate bypass** for short-but-complete pages; **synced 520-530**
  into is_status_blocked; **diagnose.py --api-url** is the canonical red-line tool (diagnose_3way doesn't
  take the fixture); **quality_gate.py gains a per-id markdown_len-drop hard gate**; **N raised to
  200-300** + min/max noise band; **89-vs-89.7 right-sized** to within-noise; concurrency fixes (arms call
  renderer.fetch not fetch_inner — permit deadlock; ghost-slot scopeguard; **RaceCancelled** breaker
  outcome + cancel token; hedge/pool lock order); **max_hedge_sessions fixed 8/not pool_size**;
  **auto-egress result through best-result-wins**; added **consume-ratio telemetry, env kill-switch,
  shadow→canary→full rollout**. **Codex (same iteration) added 4 criticals, folded in:** grace must
  NOT truncate the baseline (low-quality arm never short-circuits serial recovery; fast-accept only on
  completeness-passing results); 404/410/412/451/500 keep baseline JS escalation (excluded only from
  the egress arm, not from racing); `REQUEST_PROXY` task-local must be `.scope()`-wrapped per spawned
  arm (BYOP); `PoolGuard::Drop` is detached-spawn not a bounded reaper → add a cancellation stress
  test; per-attempt start/end timing in telemetry to prove the p90 hypothesis; scorer returns reason
  flags not just a number.
- **Iteration 2** — **CONSENSUS** (all 5 reviewers + Codex). Codex: `[CONSENSUS] Plan is ready`. Final
  residuals folded: 🔴 the completeness-bypass guard now covers **all six** cap predicates (≤10 block
  set AND ≤30 escalate set incl. `needs_js_rendering` — else a thin SPA shell false-accepts);
  `body_visible_text_len` lowercasing contract; anti-bot axis + ≤10 cap share the same four predicates;
  520-530 synced at both quality.rs + lib.rs:1195; `fast_acceptable` set by running the completeness
  predicate (not score≥70 alone). Added **Phase 1.5 read-telemetry kill-gate** (don't build the race
  if per-attempt timing shows the tail is final-Chrome-nav, not stacked-tier — convert build-then-
  revert into never-build). Concurrency: C2 scopeguard is two-path (health-check→mark-dead vs
  create-ctx→close-conn); `REQUEST_PROXY` scope must wrap renderer-SELECTION not just the arm; grace
  is a two-phase loop (expiry stops spawning, not joining); `RaceCancelled` maps directly bypassing
  `classify_outcome`. Explicit canary RAM-peak threshold ((pool8+hedge8)×per-ctx vs 3g). No 🔴/🟡
  blocking remain; residuals are implementation-test notes assigned to their phases.
