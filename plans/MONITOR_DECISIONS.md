All key anchors verify: AGPL-3.0 license, in-memory crawl_jobs with TTL cleanup, no SQL deps in opencore, BroadcastJob lease idiom, and the credit-state machinery. I have everything needed to write the decision log.

# /monitor — Architectural Decision Log

Captures the architectural debates resolved across 4 review rounds while planning the Firecrawl-parity `/monitor` feature across **crw-opencore** (Rust/Axum stateless engine, AGPL-3.0) and **crw-saas** (Next.js + Prisma + Postgres, proprietary). Each entry records the question, options considered, the decision, and the rationale grounded in the real trees.

---

## 1. Control-plane placement

**Question.** Where does the multi-tenant monitor control plane (persistence, scheduling, state machine, diff/judge orchestration, notifications, credit accounting) live — entirely in SaaS, entirely in opencore, or hybrid?

**Options.**
- (a) Entirely in SaaS; opencore gains only a stateless changeTracking/diff/judge primitive.
- (b) Entirely in opencore by adding a DB (Postgres/SQLite/redis) for monitors, snapshots, schedules.
- (c) Hybrid: control plane in SaaS, stateless primitives in opencore, plus an optional feature-flagged self-host monitor mode in opencore.

**Decision.** **Hybrid (c).** The control plane lives entirely in **crw-saas** (Prisma/Postgres + S3/R2 persistence, Vercel-cron scheduling, diff/judge orchestration, webhooks, email, credits). **opencore** gains only stateless primitives — a `changeTracking` scrape format, a `POST /v1/change-tracking/diff` endpoint, and a stateless LLM judge — and stores nothing on the hosted path. A separate, default-OFF Cargo `monitor` feature (SQLite-backed `crw-monitor` crate) gives self-hosters reduced-parity monitoring.

**Rationale.** Verified against the trees: opencore's `AppState` holds only config/renderer/in-memory `crawl_jobs`/semaphore/searxng/url_filter, with a 60s TTL cleanup loop (`JOB_CLEANUP_INTERVAL`, state.rs:80; `job_ttl_secs`, state.rs:172) — confirmed in-memory `Arc<RwLock<HashMap<Uuid, CrawlJob>>>` and **no `sqlx`/`rusqlite`/`diesel`/`sea-orm` anywhere in the workspace** (grep returned zero matches). Option (b) would force a durable DB dependency onto a tool explicitly designed to be lightweight and self-hostable — rejected. crw-saas already owns every control-plane capability except scheduling and outbound webhooks (Postgres+Prisma, atomic credits in `usage.ts`, SES, BYOK, `crwFetch`), so (a)/(c) reuse that maturely. Hybrid was chosen over pure-(a) because self-hosters of the AGPL engine still want monitoring without the proprietary SaaS, which the feature-flagged mode serves without burdening the default build.

---

## 2. Diff-engine placement

**Question.** Who computes the diffs (unified markdown git-diff + AST, and json-mode per-field path diff) — opencore Rust or saas TypeScript?

**Options.**
- (a) Rust in opencore, as a stateless primitive (`crw-diff`).
- (b) TypeScript in saas, alongside the orchestration.
- (c) Duplicated in both (Rust for hosted, TS for self-host) — no, that inverts the self-host story.

**Decision.** **Single Rust implementation in opencore — new `crw-diff` crate (a).** Pure, synchronous, no I/O, no LLM. `git_diff.rs` builds the AST directly from `similar`'s op stream (not by re-parsing a unified-diff string), `json_diff.rs` walks `serde_json::Value` to RFC-6901-ish paths, `snapshot.rs` is the single source of truth for normalization + `content_hash`. The crate **must not** depend on `crw-extract`.

**Rationale.** The diff is CPU-bound work that must behave **identically** on the hosted path and the self-host path; implementing it once in Rust guarantees parity and avoids a TS/Rust drift surface. Putting it in opencore also lets the self-host `crw-monitor` mode reuse it in-process. The `crw-extract` exclusion is deliberate: depending on it would pull the LLM/HTTP stack into a crate meant to be pure, so the judge is injected upstream in the orchestration layer. The `similar` crate is a genuinely new dependency (verified absent from `Cargo.lock`); the AST is synthesized from `DiffOp`/`ChangeTag` grouping because there is no `parse-diff` Rust crate and re-parsing our own unified output would be fragile — and both the `text` and `json` surfaces derive from the same op stream so they can never disagree.

---

## 3. Scheduler / queue approach on serverless

**Question.** How to schedule many monitors reliably on Vercel serverless — granularity, fan-out, overlap avoidance (`skipped_overlap`), load spread, and crawls that exceed the function timeout — given there is no cron/queue today?

**Options.**
- (a) Long-lived loop per the existing `broadcast.ts` pattern (a worker holds the invocation and processes to completion).
- (b) Tick-resumable model: minute-granularity Vercel crons (dispatcher + worker), durable check state machine, each invocation does one bounded unit and releases.
- (c) External queue/worker (BullMQ/SQS + dedicated worker).

**Decision.** **Tick-resumable (b),** explicitly rejecting the `broadcast.ts` long-lived-loop model. `vercel.json` (net-new — none existed) declares minute-cadence `dispatch` + `worker` crons plus a daily `retention` cron; both worker phases declare `maxDuration = 300`. The dispatcher selects `ACTIVE AND nextRunAt<=now()`, enforces an overlap guard via `currentCheckId` (insert `SKIPPED_OVERLAP` if still in-flight, else create `QUEUED` and advance `nextRunAt`). The worker atomically claims checks via the `BroadcastJob` lease idiom and **self-loops claim→process→repeat within a time budget** (`MONITOR_WORKER_CHECK_BUDGET_MS` ~200s), doing exactly one bounded unit per check (scrape batch, or crawl **kick** / **poll-once**) and never blocking on a crawl. Thundering herd is spread via `hash(monitorId) % intervalSeconds`; min interval 15m.

**Rationale.** Long crawls can exceed any function timeout, so a single invocation must never block on a crawl — this is the core reason the long-lived-loop model was rejected. Vercel does not fan one cron path into parallel invocations, so a fixed "5 checks then exit" cap would bottleneck at ~5 checks/min and form a backlog (SCALE: 200 monitors at 15m ≈ 13 checks/min steady-state); self-looping within budget removes that cliff. The `BroadcastJob` lease idiom is reused rather than invented — verified `leaseExpiresAt`/`workerPid` columns and the `updateMany`-then-`count===1` claim pattern already exist (schema.prisma:106, leaseExpiresAt:121, workerPid:122). Option (c) (external queue) was rejected as unnecessary infra given Vercel cron + Postgres leasing suffice at the assumed Pro+ (300s) ceiling; the Hobby (10s) ceiling is flagged as a re-evaluation trigger in open follow-ups. Cron-ordering between dispatch/worker is explicitly tolerated: the worker is a no-op when nothing is queued, so a missed-order tick costs at most one minute.

---

## 4. Snapshot storage

**Question.** Where are page snapshots and diffs stored — Postgres inline, object storage, or a mix — given large pages and TOAST/row-budget concerns?

**Options.**
- (a) All inline in Postgres (`@db.Text` / `Json`).
- (b) All in object storage.
- (c) Threshold split: small inline, large offloaded to S3/R2.

**Decision.** **Threshold split (c).** Snapshots ≤256 KB stored inline (`markdown @db.Text`, `snapshotJson Json`); above that offloaded to S3 (`s3Key`), and **large `diffText` is independently offloaded** to `diffS3Key` above the same threshold. `SAME` pages keep `markdown` null and reuse the prior page's `s3Key` (narrow rows). Per-check rows are hard-bounded by `maxPages` (≤1000). Retention uses S3 lifecycle rules except where reference-counting forces explicit deletion.

**Rationale.** Pure-inline (a) risks TOAST-expansion on `changed` pages' `diffText`+`snapshotJson` and unbounded row growth; pure-object (b) wastes round-trips and indexability on the common small-page case. The split was also driven by a verified infrastructure gap: only `@aws-sdk/client-sesv2` is a dependency today; **`@aws-sdk/client-s3` is net-new**, requiring a new bucket, IAM policy, and S3 lifecycle rule (the ambient credential chain is reused, but the SDK client/bucket/policy are net-new). EXPLAIN pre-ship gates were added (latest-prior lookup with TOAST rows, keyset pagination on same-`createdAt` pages) to confirm the threshold model's index behavior survives realistic skew.

---

## 5. Judging placement + cost control

**Question.** Where does the LLM "meaningful-change" judge run, and how is its cost controlled (hosted credits vs self-host BYOK)?

**Options.**
- (a) Judge inside `crw-diff` (the diff crate calls the LLM).
- (b) Judge as a separate opencore primitive (`crw-extract/src/judge.rs`) reusing existing structured-extraction machinery, injected by the orchestration layer; SaaS decides *when* and bills.
- (c) Judge entirely in saas (TS calls the LLM).

**Decision.** **opencore primitive `crw-extract/src/judge.rs`, injected by the orchestration layer (b).** `judge_change(...)` reuses `structured.rs` machinery (promoting `call_anthropic`, `call_openai`, `truncate_md`, `validate_against_schema` to `pub(crate)`), returns a `ChangeJudgment` + `llm_usage`, and does no credit math. It is injected in `single.rs` **after** `compute_change_tracking` returns (scrape path) or in the diff endpoint (crawl path) — never inside `crw-diff`. SaaS decides when (`changed` pages only, when `goal` set and `judgeEnabled`), bills **+1 credit per changed page judged**, and caps per check at `min(changedCount, MONITOR_JUDGE_MAX)` (default 200). Self-host has no credit system, so judging uses the operator's own key with a `judge_max_pages_per_check` cap (default 200).

**Rationale.** Option (a) was rejected because it would force `crw-diff` to depend on the LLM/HTTP stack, defeating its purity. The judge is ~90% already built — `extract_structured_with_usage` exists in `structured.rs` — so reusing that machinery (b) is far cheaper than re-implementing in TS (c) and keeps a single LLM path for hosted and self-host. `ChangeJudgment` is placed in `crw-core` (not `crw-extract`) so `crw-diff` can carry `judgment: Option<ChangeJudgment>` without depending on `crw-extract`. Cost control is two-sided by necessity: hosted bills per-judged-page through the existing credit system with a hard per-check cap; self-host (no Stripe/credits) relies on BYOK plus an explicit page/token cap so a self-hoster cannot incur unbounded LLM spend. The diff is treated as untrusted input with delimiter-injection defense.

---

## 6. Webhook signing / SSRF

**Question.** How are outbound webhooks signed and protected against SSRF, given the SaaS has no outbound webhook sender today and the URLs are user-supplied?

**Options.**
- (a) Unsigned best-effort POST, no SSRF guard.
- (b) HMAC-signed with a per-monitor secret, durable retries, and an SSRF guard at both save and delivery time.

**Decision.** **(b).** HMAC signing via `X-CRW-Signature: t=<unix>,v1=<hex>` where `v1=HMAC-SHA256(secret, "<t>.<rawBody>")`. Secret is `crypto.randomBytes(32)`, stored **AES-GCM encrypted** under `MONITOR_WEBHOOK_KEY`, returned once on create, and **never serialized** thereafter. SSRF guard (`webhook/ssrf.ts`) runs at **both save and delivery**: resolve hostname, reject private/loopback/link-local/metadata ranges, https-only, manual redirect handling, and pin to the resolved IP (anti-rebinding). Durable retries (1m, 5m, 30m, 2h; give up at 5) drain via the worker's webhook budget phase, claimed through a bounded index scan; terminal failures become `DEAD_LETTER` with a metric and a one-time `monitor.webhook.failing` email.

**Rationale.** Unsigned/unguarded delivery (a) is a non-starter for a multi-tenant product accepting arbitrary user URLs — it invites SSRF against internal metadata endpoints and forgeable payloads. The save-AND-delivery double check plus resolved-IP pinning specifically defeats DNS rebinding (a host that resolves benignly at save time but to a private IP at delivery). Durability and DEAD_LETTER + failure email ensure delivery failures are observable rather than silent, matching Firecrawl's durable-webhook expectation. The encrypted-at-rest, returned-once secret mirrors standard webhook-secret hygiene; the serializer secret-strip is locked by a snapshot test.

---

## 7. Email double opt-in

**Question.** How are email recipients confirmed (double opt-in) while reusing the existing SES infrastructure and suppression, and what happens when recipients are omitted?

**Options.**
- (a) Send to any listed address immediately (no confirmation).
- (b) Double opt-in via hashed confirmation tokens for new recipients, with team members auto-confirmed and an omitted-recipients fallback to team members eligible for system alerts.

**Decision.** **(b).** PENDING `MonitorRecipient` rows carry a sha256-hashed `confirmToken` (~24h) created via new `createMonitorRecipientToken`/`validateMonitorRecipientToken` in `tokens.ts`; a confirm email (`monitor-recipient-confirm.tsx`) is sent through the existing `precheck`→suppression+kill-switch path, idempotent via `claimEmailKey`, with a confirm route at `/api/monitor/confirm/[token]`. **Team members are auto-`CONFIRMED`.** If `notification.emails` is omitted, change alerts go to **team members eligible for system alerts** (active, non-suppressed, opted-in), resolved at send time so team changes are reflected. Change alerts are sent only on `changed/new/removed/error` pages, suppressed if all changes are judged noise with nothing new/removed/error, capped at ≤25 CONFIRMED recipients as a single digest; bounces feed the existing SES→SNS→`/api/ses/webhook` suppression and mirror onto `MonitorRecipient.status=BOUNCED`.

**Rationale.** Sending unconfirmed (a) risks spam complaints and SES reputation damage on user-supplied addresses — the global memory note on publish cadence reflects the same spam-signal sensitivity. Double opt-in (b) reuses the mature SES/suppression/idempotency stack rather than reinventing it, satisfying Firecrawl parity. Team auto-confirm avoids friction for known-internal addresses. The omitted-recipients fallback being resolved at send time (not materialized) keeps it correct as teams change. The single-digest ≤25 cap (never one-email-per-URL) prevents a mass-removed-page check from generating an email storm.

---

## 8. Credit reservation vs reconciliation

**Question.** How are credits accounted — upfront estimate, post-hoc actual, or reserve-then-reconcile — and how does that differ for scrape vs crawl targets given crawls discover their page count at run time?

**Options.**
- (a) Charge actuals only after the check completes.
- (b) Reserve the full upper bound upfront, reconcile to actuals at the end.
- (c) Hybrid: scrape reserves the full known upper bound at create; crawl reserves only a small seed and charges incrementally per discovered page as the crawl progresses.

**Decision.** **Hybrid (c),** reusing `checkAndConsumeQuota` + `refundCredits` + the `commitLlmReserve` reserve→actual delta. **Scrape targets** reserve `1 × urlCount` (× format add-ons) + judge headroom at create and **reject at create (403)** if the wallet can't cover the upper bound (URL count ≤50 is known upfront). **Crawl targets** reserve only "seed + judge headroom" at create, then charge incrementally per poll-once tick: `newPages = (enginePagesDiffed_now − enginePagesDiffed_prev) × rendererMultiplier`, advancing the `enginePagesDiffed` high-water mark **only after store+charge commit** in the same transaction. This is a **new worker branch** (against the `"monitor"` label, with its own high-water mark), modeled on but not literally reusing `crawl/[id]/route.ts:72-104`. At the end, reconcile via the reserve→actual delta (`actual = pagesStored × perUrlCost × rendererMultiplier + judgedChangedCount × 1`), refunding/collecting the delta. Over-spend with an empty wallet consults auto-recharge first, re-reads the balance after recharge commits, and if still insufficient caps the crawl (`PARTIAL`, pages kept) and pauses — never driving the balance negative (verified F9 clamp, usage.ts:1153-1165).

**Rationale.** Pure-actuals (a) lets a check run before confirming the user can pay; pure-upfront (b) is impossible for crawls whose page count is unknown until discovery (opencore's `CrawlState.data` materializes the set at run time). The hybrid mirrors the existing crawl-route incremental billing pattern (high-water mark + `checkAndConsumeQuota`) which the team already trusts. Two explicitly-stated caveats fall out: (1) crawl monitors do **not** guarantee a full check's credits at create time (a near-empty wallet passes the tiny seed reservation and only hits `PAUSED_NO_CREDITS` mid-check) — `estimatedCreditsPerMonth` shows the `maxPages`-bounded upper bound so the user sees real exposure; (2) the `rendererMultiplier` snapshot on `MonitorCheck` at kick closes a verified revenue leak (`crawl/[id]/route.ts:60-67` bills every crawl page at 1 credit regardless of premium renderer) rather than inheriting it. Commit-then-advance ordering guarantees re-claims never re-diff or double-bill.

---

## 9. Self-host opencore monitoring story

**Question.** Self-hosted opencore users want monitoring without the proprietary SaaS — how is that served without forcing a DB dependency on the default lightweight engine, and is set-level `new/removed` even computable in-process?

**Options.**
- (a) No self-host monitoring; direct them to the hosted SaaS.
- (b) Always-on DB in opencore (rejected under decision #1).
- (c) Feature-gated, default-OFF `monitor` mode (Cargo feature) backed by SQLite, mounted only when enabled.

**Decision.** **Feature-gated `monitor` mode (c).** A new `crw-monitor` crate is an **optional dependency of `crw-server`** activated via `monitor = ["dep:crw-monitor"]`; `rusqlite`/`tokio-cron-scheduler`/`hmac` are optional deps **of `crw-monitor` itself**, so the default server build pulls none of them. SQLite tables (`monitors`/`snapshots`/`checks`/`check_pages`, WAL) plus a background tokio task tick schedules, scrape in-process, diff via the shared `crw-diff`, compute set-level `new/removed`, and optionally fire an HMAC-signed local webhook (SMTP + unsigned-local-webhook only). Judging uses operator BYOK with a `judge_max_pages_per_check` cap. A CI gate runs `cargo tree -p crw-server` (default features) and asserts `rusqlite`/`tokio-cron-scheduler`/`hmac` are absent. Reduced parity is documented: UTC-only timezone, no Stripe/credits, SMTP/unsigned hooks; hosted CLI parity (`firecrawl monitor create`) deferred, but `crw-cli`/`crw-mcp` gain `monitor` surfaces under the feature.

**Rationale.** Option (a) abandons the AGPL self-host audience; (b) violates the statelessness that decision #1 established as opencore's defining property. The feature flag (c) preserves the default engine's zero-DB footprint while still serving self-hosters — the `cargo tree` gate makes "no leak into the default build" a hard, mechanically-verified contract (chosen over `cargo build --workspace`, which would compile `crw-monitor` and defeat the check). The **coherence question was explicitly resolved**: set-level `new/removed` is computable in-process because `CrawlState.data: Vec<ScrapeData>` (verified types.rs:694) exposes the full discovered URL set per crawl, so the SQLite reconciler can store the prior set, diff the new set, and apply the same site-down gate as the SaaS reconciler. This is stated as a hard data dependency — without `CrawlState.data` carrying the complete set, self-host `removed` would be impossible. The whole boundary inherits AGPL copyleft (verified `Cargo.toml:19 license = "AGPL-3.0"`): the new primitives are AGPL, and self-host integrators wiring their own receivers are bound by network-use copyleft, while crw-saas stays proprietary because it never links the crates — it calls opencore over HTTP.

---

## Cross-cutting decision: monitor-resume coverage (resolved in later rounds)

**Question.** When a monitor is paused for credit exhaustion, how does it resume across all four credit sources, given a paused monitor runs no reads to self-trigger a refresh?

**Decision.** A **three-layer resume** with the **daily cron balance-re-check sweep as the authoritative guarantee** (layer A, ≤24h), accelerated by fast-path hooks: the `grantPurchasedCredits` post-commit hook for `manual_topup`/`auto_recharge` (B), and explicit `syncPaidCreditsStateTx` refresh-branch (C-i) and `invoice.paid` (C-ii) hooks for subscription renewals.

**Rationale.** Verified that only `manual_topup` and `auto_recharge` route through `grantPurchasedCredits` (usage.ts:797); `monthly_refill` is a lazy in-tx write inside `syncPaidCreditsStateTx`'s `needsRefresh` branch (usage.ts:267-273, ledger write:292), and `invoice.paid` grants nothing and breaks early on `subscription_create` (so C-ii covers renewals only). A passive grant hook therefore cannot cover all four sources, so the sweep — which actively forces the lazy refresh via a new `getEffectiveBalance` wrapper — is the catch-all. This introduced one genuinely new failure mode (the sweep's read-with-write side effect racing user traffic → double `monthly_refill`), mitigated belt-and-suspenders by a **partial unique constraint** `CreditLedger(userId, source='monthly_refill', creditPeriodKey)` plus a `SELECT ... FOR UPDATE` row-lock. The read-with-write side effect is documented so a future maintainer does not "optimize" `getEffectiveBalance` into a pure SELECT and silently break the renewal-resume guarantee.

---

The decision log above is the deliverable. Relevant grounding files: `/Users/us/coding/crw/crw-opencore/Cargo.toml` (license:19), `/Users/us/coding/crw/crw-opencore/crates/crw-server/src/state.rs` (in-memory jobs, TTL:80/172), `/Users/us/coding/crw/crw-opencore/crates/crw-core/src/types.rs` (CrawlState.data:694), `/Users/us/coding/crw/crw-saas/prisma/schema.prisma` (BroadcastJob lease idiom:106-122), and `/Users/us/coding/crw/crw-saas/src/lib/usage.ts` (syncPaidCreditsStateTx:250, monthly_refill:292, grantPurchasedCredits:797, commitLlmReserve:1036).
