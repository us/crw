I now have the complete plan. Producing the sign-off summary.

# /monitor Plan — Go/No-Go Sign-Off Summary

**Verdict: IMPLEMENTATION-READY. GO for M1.**

The final plan is buildable as written. Every load-bearing claim is pinned to a verified code anchor (re-verified this revision), all four previously-blocking gaps (#3 PATCH state machine, #4 cascade delete, #5 transactional opt-in, #10 consume-path credit race) are closed with concrete mitigation sites, and the security reviewer's nine-item checklist (§1.5) is renderable as PASS. No open question blocks M1 — the only deferred item (hosted CLI parity) needs product sign-off, not engineering work, and does not gate any milestone.

## Headline Architecture Decision

**Hybrid split.** opencore (Rust/Axum, AGPL-3.0) gains only *stateless primitives* — a `changeTracking` scrape format, a `POST /v1/change-tracking/diff` endpoint (single + batch), and a stateless LLM judge — and stores nothing on the hosted path. The entire multi-tenant control plane (persistence, scheduling, diff/judge orchestration, set-level `new`/`removed` reconciliation, credits, webhooks, email) lives in crw-saas (Next.js/Prisma/Postgres), which calls opencore over HTTP and never links the AGPL crates (preserving the proprietary boundary). A feature-flagged, SQLite-backed `monitor` self-host mode (default OFF) gives reduced-parity monitoring without forcing a DB dependency on the default engine.

## Six Milestones (one line each)

1. **M1 — opencore diff engine (OSS):** new `crw-diff` crate (git/json/mixed diff from `similar` ops, binary-hash, diff-size cap, mode-aware hash) + `crw-core` types (`OutputFormat::ChangeTracking` string variant, `ChangeJudgment`, `ScrapeData.content_type`) + scrape wiring + `/v1/change-tracking/diff` (batch discriminator, actionable parse errors, server-side batch cap) + capabilities advertise + dependency-direction CI gate + four `/metrics` counters.
2. **M2 — opencore judge (OSS):** `crw-extract/src/judge.rs` reusing `structured.rs` machinery (promote 4 symbols to `pub(crate)`), `goal`/`judgeEnabled` fields, judge injection in `single.rs` (`Some(true)`-only guard), config caps.
3. **M3 — SaaS data + CRUD:** Prisma models + migration (cascade FKs, partial unique index, `transactionalAlertsOptIn`), the `SELECT ... FOR UPDATE` consume-path row-lock, `/v1/monitor` CRUD + `/run` (409 on in-flight) + PATCH state machine, serializers + snapshot/cursor tests, Zod validation, DST-correct scheduling, per-plan gating, dashboard list/create/edit.
4. **M4 — SaaS scheduler + execution:** `vercel.json` dispatch/worker (tick-resumable, self-looping, lease 240s > budget 200s > unit-cap 60s + heartbeat), overlap guard, no-backfill catch-up, delete-mid-run safety, `run-check.ts` (inline scrape diff + batched crawl diff with high-water-mark commit), engine-job-lost reconciliation, site-down gate, S3 snapshot offload, incremental crawl billing, pause/resume, EXPLAIN index gates.
5. **M5 — SaaS notifications:** signed webhook delivery (HMAC, SSRF-pinned, durable retries → DEAD_LETTER), email double opt-in + digest suppression + team-eligibility fallback, retention cleanup + monitor-delete cleanup + resume sweep.
6. **M6 — opencore self-host `monitor` mode (OSS, opt-in):** feature-gated `crw-monitor` (SQLite + scheduler + local webhook + CLI/MCP, UTC-only, BYOK judge cap, set-level new/removed via `CrawlState.data`, cascade + re-baseline parity) + default-build CI gate.

## Top 5 Risks Already Mitigated

1. **Credit double-spend race (consume path):** verified vulnerable today — consume `$transaction` at usage.ts:650 runs default Read Committed with no row lock, so concurrent monitor charge + user API traffic can both pass the balance guard and drive the balance negative. Mitigated by `SELECT ... FOR UPDATE` on the user row as the first statement in that transaction (test-locked; shipped behind a mandatory p99 load-benchmark gate before fleet-wide enablement).
2. **Lease/budget mismatch → double-execution + double-billing:** lease (240s) strictly exceeds the worker check-budget (200s), with ≤30s heartbeat renewal and a 60s per-unit wall-clock cap that splits oversized scrape targets across ticks via `scrapeUrlCursor`, so a second worker's claim of an in-flight check matches zero rows.
3. **Engine job lost (in-memory 60s TTL on opencore):** explicit `ENGINE_JOB_LOST` transition → PARTIAL/FAILED with delta-refund and bounded auto-retry-once keyed on `enginePagesDiffed == 0`, so no orphaned state or double-billed progress.
4. **Crawl credit blowup:** hard `maxPages` cap + incremental per-page charge on the `enginePagesDiffed` commit-then-advance high-water mark (× snapshotted `rendererMultiplier`, closing the inherited renderer under-bill), with cap-crawl+pause as the sole over-spend backstop and F9-clamped reconcile — never negative balance.
5. **PATCH/DELETE corrupting baselines or orphaning state:** §4.2.3 update state machine (in-flight PATCH → `pendingUpdate`; baseline-invalidating change → `baselineEpoch++` → next check treated as `firstObservation`, never diffed against an incompatible snapshot) plus `onDelete: Cascade` on all child relations + in-flight abort/reconcile + immediate S3 cleanup on delete.

**Plan file:** `/tmp/plan_current.md`
