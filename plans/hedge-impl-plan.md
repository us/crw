# Conditional Hedge — Implementation Plan (latency-qn, p90→~10s)

## Goal
Race the cheap JS tier (lightpanda) and chrome **concurrently** instead of serial, so
chrome's render clock starts immediately. Measured (telemetry sim): p90 14.5→**10.4s (−28%)**,
**success/recall identical** (best-result-wins, same accept gate). This is exactly Firecrawl's
pattern (`scrapeURL/index.ts`: `Promise.race([...enginePromises, timeouts])` + per-engine
`getEngineMaxReasonableTime` → start next engine on timeout, first-good-wins) and Spider's
`race_backends` (JoinSet + grace + `html_quality_score` best-wins).

## Why it's success-safe (the red line)
The accept-gate is UNCHANGED; only attempt ORDERING/TIMING changes. best-wins ⇒ never returns a
worse result than serial. Three reviewer rules make it provably ≡ serial:

- **Rule A (accept arbitration):** among results that PASS the gate, return by fixed **tier
  priority (lightpanda > chrome)**, NOT arrival order. (Serial accepts lightpanda when it passes,
  never seeing chrome; racing must reproduce that. First-to-arrive may *cancel* the slower, but if
  both pass, lightpanda wins.)
- **Rule B (best-thin):** if neither passes, await BOTH, pick **richest HTML** (ties → tier
  priority). No first-thin early-return. (Matches serial's thin-stitch which keeps the larger HTML.)
- **Rule C (winner-only side-effects):** record breaker/preference/`saw_hard_block`/
  `last_failover_reason` **only for the returned tier** (and, in the all-thin case, the tiers serial
  would have walked). Suppress an aborted/cancelled loser's signals — else the preference learner /
  breaker / hard-block residential arm drift, a latent recall risk + needless residential latency.

## Correctness (tokio)
- Drive both with **`tokio::select!` + `tokio::pin!`** (NOT spawn) so REQUEST_PROXY/REQUEST_COUNTRY
  task-locals propagate. Per-branch `if !done` guards (polling a completed future panics).
- **Drop-cancellation:** on accept, drop the losing future → its PoolGuard Drop reaper must
  SYNCHRONOUSLY close/reset the CDP target (not just free the slot) or the next acquirer inherits a
  navigating zombie. VERIFY browser_pool reaper closes the target on drop; add a drop-mid-render test.
- **Outer bound:** wrap the race in `tokio::time::timeout_at(deadline)` so it can't hang.
- **Pool saturation (2N contexts):** hedging 2 contexts/request deadlocks at ≥4 concurrent vs
  pool=8. Gate eligibility on a `Semaphore(pool_size/2)` acquired with **`try_acquire`** (never
  block — blocking to hedge defeats the win); no permit ⇒ fall back to serial (1 context).

## Eligibility (else serial, unchanged)
auto mode (not user-pinned) · not proxy_active · renderers contain both lightpanda + chrome · NOT
already promoted-to-chrome-first (then chrome is first → serial already optimal) · hedge Semaphore
`try_acquire` ok · behind config flag `chrome_hedge` (default false, A/B-able).

## Implementation steps
1. **Extract `evaluate_attempt(result, kind, ...) -> AttemptVerdict`** from the serial loop body
   (lib.rs ~1265-1470): the accept-gate + thin/hard-block classification, returning
   `Accept(FetchResult)` | `Soft { thin, err_kind, hard_block }` WITHOUT committing side-effects
   (caller commits per Rule C). Serial path refactors to use it (behavior-preserving — verify by
   bench parity first).
2. Add `chrome_hedge: bool` config knob + `RendererConfig` field + plumb to FallbackRenderer.
3. Add hedge Semaphore (size pool_size/2) to FallbackRenderer.
4. Implement the select! race per Rules A/B/C in fetch_with_js, gated by eligibility.
5. Breakers: acquire permits for both tiers; commit outcome only for winner (Rule C); release the
   loser's probe_guard as a no-op (disarm only winner).

## Verification
- Bench A/B on clean lp+chrome ladder, challenge=1, fast_ready=on: hedge OFF vs ON.
- `quality_gate.py` MUST pass (0 new failures, 0 content drops) — the red line.
- Expect p90 ~14→~10s, success/recall flat. ≥2 runs for noise band (fixture has flaky anti-bot URLs;
  mtkxjs is load-sensitive — run at low system load).

## Secondary levers (noted, not in this change)
- Session/page reuse beyond context-reuse: marginal (pre-nav overhead already ~200-500ms amortized).
- auto_scroll/auto_click already exit early (p90 741ms) — not a lever.
- The residual gap to FC's 6.9s is fire-engine's tuned persistent render (proprietary infra).
