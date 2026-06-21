# Readiness v2 — quality-gated content-ready exit (FC-style, non-truncating)

## Problem
fast_ready v1 exits on `(body_text≥800) OR (body_text≥200 AND networkAlmostIdle)`, ceiling 8s. On
tracker-busy / dynamic-text JS pages networkAlmostIdle never fires → 8s ceiling burn (columbusjack,
ericciarla). Every aggressive early-exit tried (DOM-stable text-length, first-party-idle, DCL,
timeout↓) TRUNCATED content because each used a SINGLE GLOBAL signal conflating main content with
peripheral noise (text→clocks/counters; network→trackers; first-party→3rd-party-CDN content).

## Research finding (multi-agent: commercial services + SOTA)
Fast services (FC ~1.5-2.5s, browserless `bestAttempt`, Prerender, Rendertron) RACE: snapshot at the
first of {DCL+short-quiet, target predicate, app-ready flag}, ceiling = fallback only. The robust
NON-TRUNCATING generic predicate = **scope the signal to the content region + gate on content QUALITY**:
- **Content-quality gate (anti-truncation):** never exit until extracted main-text ≥250 chars AND
  link-density <0.2 (trafilatura MIN_EXTRACTED_SIZE 250 / jusText link-density 0.2). THE missing piece.
- **Region-scoped DOM-quiet (anti-dynamic-text):** observe only main/article/largest-text-block (not
  body), drop `characterData` (clock digit flips don't reset), ~800ms settle.
- **CPU-idle OR network-idle (anti-tracker breakthrough):** no Long Task ≥50ms for ~500ms = render
  done even while trackers keep the network busy (Lighthouse TTI uses this). CPU-idle is what defeats
  trackers that network-idle can't.
- **LCP-stable** optional vote; **hard cap** backstop.

## Design — `eval_content_ready` (replaces the fast_ready poll)
A single injected Promise (Runtime.evaluate, awaitPromise:true) that resolves `"ready"` when the v2
predicate holds, `"cap"` at its internal ceiling. Browser-side observers (more accurate than 200ms
Rust polling, 1 round-trip vs many):
- pick region = largest innerText among `main, article, [role=main], #root, #app, #__next` else body.
- quality() = region text ≥ MIN_CHARS(250) AND link-density <0.2.
- MutationObserver on body `{childList,subtree,attributes}` (NO characterData) → `lastMutation`.
- PerformanceObserver `{type:'longtask'}` → `lastLongTask` (try/catch; unsupported → CPU treated idle).
- tick (100ms): ready when `quality() AND (now-lastMutation≥800) AND (now-lastLongTask≥500) AND
  readyState==='complete'`; cap at spa_max.
Returns bool (ready vs cap); caller snapshots either way (cap = best-effort, same as v1 ceiling).

Why it beats v1 + the failed levers:
- columbusjack login page: form passes quality fast → exit ~1.5s (FC-like).
- dynamic-text page: region-scope + no characterData → clock doesn't reset settle.
- tracker-busy page: CPU-idle fires though network never idles.
- late-burst / thin-shell: quality gate not met → wait (no truncation). ← the guard all prior levers lacked.
- sub-250-char real pages (example.com 167): quality never passes → cap (slow but CORRECT, == v1 today).

## Implementation
- cdp.rs: new `async fn eval_content_ready(conn, session_id, timeout, spa_max) -> bool` with the probe.
- wait_for_spa_selector: `if fast_ready { return Self::eval_content_ready(...).await; }` then legacy poll.
- Tunable consts: READY_MIN_CHARS=250, READY_MAX_LINK_DENSITY=0.2 (×100 int), READY_DOM_QUIET_MS=800,
  READY_CPU_QUIET_MS=500, READY_TICK_MS=100. (No new config flag — gated by existing chrome_fast_ready;
  A/B = fast_ready off vs on.)

## Verification
A/B fast_ready(v1 baseline=prior runs) vs v2, + quality_gate (MUST hold — the red line; prior levers
failed here). Multiple runs (fixture flaky + machine noisy). Expect: tracker-busy/login pages drop
from ~8-13s toward ~2-3s, content held (quality gate). Local bench noise-limited → prod canary for
authoritative numbers. Multi-agent review the probe JS before shipping.
