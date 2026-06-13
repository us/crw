# crw-mcp Optimization — Implementation Plan

> Goal: make `crw-mcp` a best-in-class MCP server — minimal context/token footprint,
> lightest-possible runtime, frictionless install on any agent, and clean MCP design.
> Worktree: `/Users/us/coding/crw/crw-opencore-mcp-optimize` (branch `feat/mcp-optimization`).

## Context

The fastCRW MCP server (`crates/crw-mcp`, with shared protocol in `crates/crw-mcp-proto`,
the embedded backend in `crates/crw-server/src/routes/mcp.rs`, and a near-duplicate path in
`crates/crw-cli/src/commands/mcp.rs`) is functional and Firecrawl-API-compatible. It exposes
6 tools: `crw_scrape`, `crw_crawl`, `crw_check_crawl_status`, `crw_map`, `crw_search`,
`crw_parse_file`. It already advertises MCP revision `2025-06-18` and emits `structuredContent`.

It is not optimized for the three things that decide whether an agent enjoys an MCP server in
2026: **how many tokens it costs just to exist in context**, **how heavy it is to run/ship**,
and **how painful it is to attach**.

**Verified facts (research + code audit, cited):**
- A tool's name + description + full `inputSchema`/`outputSchema` is injected into the agent's
  system prompt **every turn**; per-tool cost ~100–1,000 tok; 20–30 tools cost 15–30 KB before
  the first user message ([MCP #2808](https://github.com/modelcontextprotocol/modelcontextprotocol/issues/2808)).
- **Our `tools/list` is ~13–14 KB ≈ 3,250–3,500 tok.** `crw_search` alone ≈ 700 tok: a 926-char
  description (~231 tok, `crw-mcp-proto/src/lib.rs:205`) + a verbose `$defs`/`oneOf` `outputSchema`
  (~450 tok, `:272-319`), including meta-prose ("`snippet` is an alias of `description`").
- Tool results are emitted as a **pretty-printed** text block **and** as `structuredContent`
  (`lib.rs:446-461`). Pretty-printing adds ~30% whitespace. Dual emission is **spec-recommended,
  not a violation** — see Decision D1.
- **No output bounds anywhere.** `crw_map` can dump up to `MAX_DISCOVERED_URLS = 5000`
  (`crw-crawl/src/crawl.rs:17`) with no `limit` in the MCP schema (`lib.rs:169-195`). `crw_scrape`
  serializes full markdown, no cap (`routes/mcp.rs:26-52`). **`crw_check_crawl_status` returns
  every crawled page's markdown/html/rawHtml** — the single largest unbounded dump (`routes/mcp.rs:61-73`).
  Claude Code hard-caps tool results at ~25K tok and spills the rest to disk
  ([claude-code #45770](https://github.com/anthropics/claude-code/issues/45770)) — unbounded output is
  silently lost, not helpful.
- **No tool annotations** (`readOnlyHint`/`destructiveHint`/`idempotentHint`/`openWorldHint`,
  MCP 2025-03-26). Their JSON defaults are hostile: `destructiveHint` defaults to **true**, so
  omitting them makes read-only tools look destructive → spurious client confirmations.
- **Runtime weight: `crw-renderer` (auto-browser) is a non-optional dependency**
  (`crates/crw-mcp/Cargo.toml:22`), so it compiles into **proxy mode too** — only `crw-server`
  is feature-gated (`:32`). The embedded path also auto-spawns a headless browser (150–500 MB
  RAM, 0.5–2 s start; `main.rs:383-408`). The lean-proxy story is blocked by this dep, not just
  by the default feature set.
- The release profile already sets `lto = true`, `codegen-units = 1`, `strip = true`
  (`Cargo.toml:147-150`). It does **not** set `opt-level="z"`/`panic`. `panic = "abort"` is
  **unsafe here**: `crw-extract/src/pdf.rs:214` uses `catch_unwind` to survive malformed-PDF
  panics; under abort a crafted PDF crashes the whole server (Decision D4).
- **Install scaffolding already exists**: `mcp/crw-mcp/package.json` + `bin/crw-mcp.js` implement
  the npm `optionalDependencies` per-platform wrapper with a GitHub-download fallback; a root
  `server.json` (name `io.github.us/crw`) already feeds the MCP registry. Phase 6 is mostly
  *finishing/verifying*, not building from scratch.

**What we are NOT doing (scope guard):**
- Not adding Firecrawl's 20-tool surface (`batch_scrape`, `extract`, `agent`, `monitor_*`).
  Tool *count* stays ≤ 6.
- **Lazy/dynamic tool exposure (a "tool search" meta-pattern) is explicitly deferred.** It is
  the single biggest token lever for *large* servers (50–90% cuts), but at 6 small tools the
  payoff is modest, Claude Code already auto-activates native tool-search when schemas exceed
  ~10% of context, and a meta-tool indirection hurts one-shot tool selection. Revisit only if
  the tool count grows. (Recorded here so its absence is a decision, not an oversight.)
- Not changing the REST/HTTP API contract or `crw-core` types. All output bounding is done at
  the **MCP layer** (Decision D2) — no new fields on `ScrapeRequest`/`ScrapeData`/`MapRequest`.
- Not rewriting the scraping engine, crawler, or renderer.
- Not switching the protocol revision away from `2025-06-18`.
- **Not flipping the default mode** from embedded → proxy in this work (Decision D3). We make a
  lean proxy build *available*; we do not break zero-config users.
- **Not re-implementing SSRF/URL-safety in the proxy binary.** Verified: the embedded MCP path
  validates URLs (`routes/mcp.rs:21` → `validate_safe_url_resolved`), and **every REST endpoint
  the proxy forwards to also validates** (`routes/scrape.rs:19`, `v2/{scrape,crawl,map,batch,
  extract}.rs`). So SSRF is enforced server-side in both modes; the proxy binary intentionally
  does not duplicate it (the upstream is the trust boundary). Flagged here so it's a decision, not
  a blind spot — if a non-crw upstream is ever supported, revisit.

## Decisions (resolve the blocking open questions up front)

These were Open Questions in v0; reviewers (Codex + spec + token + compat) flagged that several
are blocking and must be settled before Phase 1 touches schemas/tests.

- **D1 — `content` vs `structuredContent` (was Q1).** The MCP spec *recommends* emitting both:
  if an `outputSchema` is declared, results MUST conform to it and SHOULD also be serialized as
  a TextContent block for back-compat
  ([spec: server/tools](https://modelcontextprotocol.io/specification/2025-06-18/server/tools)).
  So dual emission is **not** a violation, and the v0 claim "clients SHOULD NOT forward both" was
  a misattribution (that's community discussion [MCP #1624](https://github.com/modelcontextprotocol/modelcontextprotocol/issues/1624),
  not normative). **Decision:** keep dual emission; the real, client-independent wins are (a)
  **minify** the text block (`to_string`, not `to_string_pretty`) and (b) **slim the
  `crw_search` `outputSchema`** (drop per-field descriptions + `$defs` prose; keep a skeletal
  conformant shape) — this saves ~200–300 tok in `tools/list` every turn regardless of how any
  client forwards results. **Before** changing result emission further, empirically check what
  Claude Code does with a result that has both fields (the *empirical check* below). If it
  double-counts → drop the text block for schema-bearing tools; if it suppresses one → leave as
  is. Spec note (corrected from v1): emitting `structuredContent` *without* an `outputSchema` is
  fully legal; the **only spec-illegal combination is declaring an `outputSchema` and emitting
  `structuredContent` that does not conform to it** (the spec says results MUST conform when a
  schema is declared). So a slimmed `outputSchema` must still validate every real flat/grouped
  `crw_search` response (locked by test T4).
  - **Empirical check (do at the end of Phase 1, before Phase 2 result changes):** attach the
    server to Claude Code, call `crw_search` (the only schema-bearing tool), and inspect whether
    the result's `content` text counts toward conversation tokens when `structuredContent` is
    present (via `/context` or transcript inspection). Record the finding in the PR; it decides
    whether Phase 2 drops the text block for `crw_search`.
- **D2 — Truncation lives at the MCP layer, not in `crw-core`.** Honors the scope guard. Bounds
  are MCP-only arguments (`limit`, `maxLength`) parsed from `args` and applied by slicing the
  already-returned value before serialization, plus consistent markers. No `crw-core`/`crw-crawl`
  type changes. (For `crw_map`, slicing after `discover_urls` returns keeps `totalDiscovered`
  knowable; we accept that discovery still does full work — bounding context, not runtime.)
  - **Bound semantics (resolve the v2 contradiction):** **omitted = conservative bounded
    default**; **`limit: 0` = explicit unbounded opt-out**. Schema documents this (`minimum: 0`,
    description "0 = unbounded"). Same pattern for `maxLength` (omitted = default cap, `0` = no cap).
  - **MCP-only args must be stripped before proxy/REST forwarding.** `limit`/`maxLength` are MCP
    controls; a stricter/older upstream may reject unknown body fields. The proxy + cli paths
    extract these locally, remove them from the JSON sent to `/v1/*` `/v2/*`, then apply bounds
    to the *response*. (Embedded path: `serde` already ignores them on the typed request; still
    apply bounds to the result.)
  - **Bound helper signature:** a single `apply_bounds(tool_name, args, value) -> Value` living
    next to `tool_result_response` in `crw-mcp-proto`, **non-mutating** (never writes back to the
    stored crawl-job state), reused by embedded, proxy, and cli for parity.
  - **`crw_scrape` MCP output projection (make it explicit, not "{markdown,url,title}-class"):**
    the default MCP result projects exactly `{ markdown?, html?, links?, url, title, statusCode,
    truncated? }` from `ScrapeData` and drops heavy/debug fields (`rawHtml`, `screenshot`, full
    `metadata`, renderer/cost debug). `formats`/an explicit `include` opts heavier fields back in.
    See Versioning for the back-compat classification of dropping `rawHtml`-by-default.
- **D3 — Default stays `embedded`; lean proxy becomes a first-class build.** Flipping the default
  would silently break every zero-config user (incl. the `crw-saas` prod deploy). Instead,
  feature-gate `crw-renderer` behind `embedded` so a `--no-default-features` proxy build is tiny,
  and document/ship it as the lean option. Any future default flip goes through a `CRW_MODE`
  env with a one-release deprecation window — out of scope here.
- **D4 — No `panic = "abort"`.** It breaks the `catch_unwind` malformed-PDF safety valve
  (`crw-extract/src/pdf.rs:214`) and could leak browser child processes on panic
  (`main.rs:422` drops guards only on the normal path). The build-size delta from abort is small;
  not worth the crash risk. Size work uses `opt-level` + dependency gating only.
- **D5 — Keep all current input params; trim prose only (was Q3).** Removing `includeTags`/
  `excludeTags`/`categories` from schemas would degrade client-side validation/autocomplete for
  introspecting consumers (n8n nodes, IDE UIs) even though `serde` tolerates extras
  (no `additionalProperties:false`, `lib.rs:585`). Phase 1 shortens *descriptions*, it does not
  remove parameters.

## Approach

1. **Cut context cost first** (highest leverage, lowest risk, mostly one file): trim every
   description to one decisive sentence, slim the `crw_search` `outputSchema`, minify result
   text. Add a tokenizer-based budget test + a tool-selection eval.
2. **Bound every result path** at the MCP layer with a single shared truncation helper, opt-out
   semantics, conservative defaults, enforced in **both** embedded and proxy modes (and the
   `crw-cli` path), including the worst offender `crw_check_crawl_status`.
3. **Add correct tool annotations** and tighten protocol conformance (title placement, header
   handling, unknown-tool semantics).
4. **Make a genuinely lean proxy binary** by feature-gating `crw-renderer`, adding `opt-level`,
   and measuring with platform-correct tooling.
5. **Ship the lean build + finish the (already-scaffolded) distribution**, flagging the real
   blockers (Apple signing) honestly.

**Trade-offs accepted:** shorter descriptions risk tool-selection regressions → mitigated by a
20-prompt eval; truncation can hide content → always marked + opt-out; lean proxy needs a
reachable API/key → documented, default stays zero-config embedded.

## Phases

### Phase 1 — Slash the tool-definition footprint (descriptions + outputSchema)
**Files:** `crates/crw-mcp-proto/src/lib.rs` (`tool_definitions` ~68-356; tests ~476-776).
- Rewrite every tool `description` to one decisive sentence; remove return-shape prose, examples,
  and alias meta-comments from `crw_search` (`:205`). Shorten each property description to ≤ ~8
  words. **Keep all params** (D5).
- Slim `crw_search` `outputSchema` (`:272-319`): drop per-field `description`s and `$defs` prose,
  keep a minimal shape that real flat/grouped responses still validate against (D1). Keep the
  T4 schema-validation test green.
- Add a **token-budget test** using a real tokenizer: `tiktoken-rs` as a **dev-dependency**
  (does not bloat the release artifact — compiled only under `cargo test`; verify it builds in CI
  with no extra toolchain before gating on it — it vendors its BPE data, no Python needed). If the
  tokenizer is unavailable, fall back to `ceil(chars/3)` (over-counts → conservative for a ceiling
  test; consistent with Verification). **Target ≤ 1,000 tok** (estimated floor for 6 trimmed
  tools ~600–900 tok). Set the **CI failing ceiling to the measured post-trim floor + ~15%
  margin**, not the aspirational 1,000 — so the gate catches regressions without churning on
  noise. Record before/after in the PR.
- **Verified (no action, document it):** the `initialize` response emits only `protocolVersion`,
  `capabilities`, and `serverInfo{name,version}` (`crw-mcp-proto/src/lib.rs:401-414`) — **no
  `instructions` field**, so there is no per-session instruction-string token cost to trim. If
  an `instructions` string is ever added, it must be counted against the same budget.
- Add a 20-prompt **tool-selection eval fixture** (scrape vs map vs crawl vs search vs parse)
  run before/after trimming to catch selection regressions. Pin it for reproducibility: fixtures
  in `tests/eval/`, graded by exact expected-tool-name match, run behind `--ignored` (manual /
  nightly, not blocking CI), with the judging model + prompt template recorded in the fixture.
- **Description ↔ runtime-dependency tension:** the trimmed `crw_search` sentence must still hint
  that it needs a search backend (it fails `search_disabled` with no SearXNG), without a paragraph.
  Suggested: *"Search the web (needs a configured search backend; embedded uses a local SearXNG
  sidecar)."* — one sentence that preserves the signal. (See Phase 3 conditional advertisement.)
- **Effort:** M. **Risk:** Med (selection quality; mitigated by eval).

### Phase 2 — Bound & compact every result path (MCP layer)
**Files:** `crates/crw-mcp-proto/src/lib.rs` (`tool_result_response` ~439-474; add a shared
truncation helper); `crates/crw-server/src/routes/mcp.rs` (`call_tool` 24-145);
`crates/crw-mcp/src/main.rs` (proxy dispatch — bounds enforced locally after `parse_response`);
`crates/crw-cli/src/commands/mcp.rs` (the duplicate path — update or refactor to shared code).
- Replace `to_string_pretty` with `to_string` for the text block (`lib.rs:446`); test asserts no
  pretty whitespace.
- **One shared truncation helper** applied consistently to: `crw_scrape` markdown/html/rawHtml,
  `crw_parse_file` markdown, `crw_search` (result count + any `scrapeOptions`-inlined content),
  `crw_map` links, and **`crw_check_crawl_status`** page contents. Marker is uniform:
  `truncated: true` + `returnedLength`/`originalLength` (or `returned`/`totalDiscovered` for
  list-shaped results), never a silent cut.
- **MCP-layer args (D2):** add `limit` (lists) and `maxLength` (text, chars) to the relevant
  input schemas; parse from `args`, slice the returned value. **Bound semantics (per D2):**
  *omitted* = conservative bounded default; **`limit: 0` / `maxLength: 0` = explicit unbounded
  opt-out** (preserves today's unbounded behavior for callers who ask for it). Schema documents
  this (`minimum: 0`, description "0 = unbounded"). **Conservative defaults:** `crw_map` limit
  ≈ 100; `crw_scrape`/parse `maxLength` ≈ 12–15K chars (~3–4K tok, well under the 25K client
  cap) — callers opt *in* to bigger, not out of small.
- **Field selection:** `rawHtml`/`html`/`links` are already `formats`-gated in extraction
  (`crw-extract/src/lib.rs`) — they only appear when requested, so they are *not* the problem.
  The real waste is the **always-serialized `metadata`** (and any renderer/cost/debug fields) on
  `ScrapeData`. Phase 2 projects the MCP result to `{ markdown?, html?, links?, url, title,
  statusCode, truncated? }` (per D2) and drops always-present `metadata`/debug from the default
  MCP shape; a client can opt heavy fields back in. (Bigger result-token win than `maxLength` alone.)
- **Enforce in proxy mode locally:** `crw-mcp` forwards to `/v1/*`; a remote/older server may
  ignore the new MCP-only params, so the proxy path must strip the MCP-only args from the
  forwarded payload and post-process/truncate responses itself (`main.rs`), not assume the
  upstream did. Same for `crw-cli`.
- **Close the existing `crw-cli` parity gap (verified bug):**
  `crates/crw-cli/src/commands/mcp.rs:190` falls through to `unknown tool` for **`crw_parse_file`**
  — it's advertised in the shared `tools/list` but unimplemented in the cli proxy dispatch (only
  `crw_scrape`…`crw_search` exist, no `crw_parse_file` arm). Add the missing arm as part of this
  phase, or the cli MCP advertises a tool it can't serve.
- **Bound stderr debug logging too:** `main.rs` logs the full response JSON at `debug`
  (`tracing::debug!("→ {out}")`, ~`:463/481`) — a 50-page `crw_check_crawl_status` result becomes
  a multi-MB log line under `RUST_LOG=debug`. Log byte length (or a truncated prefix), not the
  whole payload. (Operational footgun, separate from context tokens.)
- **Shared-code strategy (clarify effort):** the cli path (`commands/mcp.rs`) is a ~400-line
  near-copy of `crw-mcp/src/main.rs`'s proxy dispatch. Preferred: extract the proxy dispatch +
  `apply_bounds` into a shared module in `crw-mcp-proto` (or a small shared crate) consumed by
  both — this removes the drift class entirely (`crw_parse_file` gap above is exactly this drift).
  If a full extraction is too large for one PR, copy `apply_bounds` into both with a shared test,
  and file a follow-up to dedupe. State which path was taken in the PR.
- **Effort:** L–XL (6 files incl. `crw-cli` + `teardown.rs`; shared-helper extraction can push to
  XL). **Risk:** Med — default bounds change observable behavior (see Versioning); covered by
  tests + opt-out.

### Phase 3 — Correct tool annotations & protocol conformance
**Files:** `crates/crw-mcp-proto/src/lib.rs` (`tool_definitions`, `handle_protocol_method`);
`crates/crw-server/src/routes/mcp.rs` (HTTP header handling).
- Annotations per tool (explicitly emit every value — defaults are hostile):
  - `crw_scrape`, `crw_map`, `crw_search`, `crw_check_crawl_status`, `crw_parse_file`:
    `readOnlyHint:true, destructiveHint:false, idempotentHint:true, openWorldHint:true`
    (`openWorldHint:false` for `crw_parse_file` — it reads provided bytes, not the open web).
    Note for reviewers: `idempotentHint` concerns *side-effects*, not result determinism — a
    live `crw_search`/`crw_check_crawl_status` may return different bytes over time yet is
    side-effect-free, so `idempotentHint:true` is correct per spec.
  - **`crw_crawl`: `readOnlyHint:false`** (starting a job is a side effect — spec + Codex
    consensus), `destructiveHint:false, idempotentHint:false, openWorldHint:true`.
- Add a top-level **`title`** to each tool (`Tool.title` from `BaseMetadata`, the preferred
  display field — **not** `annotations.title`).
- HTTP transport: per the 2025-06-18 transports spec, an **invalid/unsupported**
  `MCP-Protocol-Version` **SHOULD** get `400` (not a hard MUST); a *missing* header SHOULD assume
  `2025-03-26`. Today `routes/mcp.rs:214` only reads the header. Decide: keep lenient (document
  why) or add the 400 branch for invalid values; update the header-tolerance test either way.
- Unknown-tool semantics: today it's `isError:true` text (`routes/mcp.rs:170`, `lib.rs:466`).
  **Decision:** switch unknown-tool to a JSON-RPC `-32602` protocol error (clients degrade more
  gracefully than on an `isError` text blob); tool-*execution* failures correctly stay
  `isError:true`. Lock both with tests. (Spec doesn't mandate either, but `-32602` is the
  better-supported convention for "this tool doesn't exist".)
- `capabilities`: `tools:{}` is legal; consider `tools:{listChanged:false}` for explicitness.
- **Conditional `crw_search` advertisement (frictionless-install fix):** today `crw_search` is
  *always* in `tools/list` (`lib.rs:197`), but in embedded mode with no `[search].searxng_url` it
  returns `search_disabled` (503) at call time (`config.rs:184`, `routes/search.rs:160`) — a
  footgun for `npx -y @crw/mcp` users who see the tool and get an error. Suppress `crw_search`
  from `tools/list` when the embedded backend has no search backend configured (proxy mode keeps
  advertising it — the remote decides). Threading the config into `tool_definitions` is a small
  signature change; alternatively keep advertising it but make the `search_disabled` error
  actionable ("set [search].searxng_url or run the SearXNG sidecar: …"). Decide; document the
  SearXNG sidecar in the install docs (Phase 6) either way.
- **Effort:** S–M. **Risk:** Low.

### Phase 4 — Lean runtime: feature-gate the renderer + size profile
**Files:** `crates/crw-mcp/Cargo.toml` (the real lever); workspace `Cargo.toml` (profile/deps).
- **Feature-gate `crw-renderer` behind `embedded`** (`crw-mcp/Cargo.toml:22`) so a
  `--no-default-features` (proxy) build excludes the auto-browser dependency entirely. This,
  not the profile, is what unlocks a small proxy binary. Wiring:
  `embedded = ["dep:crw-server", "dep:crw-renderer", "crw-renderer/auto-browser"]`, with the
  `use crw_renderer::browser;` in `main.rs:40` (already `#[cfg(feature="embedded")]`) unchanged.
- **Hard prerequisite (verified blocker):** `crates/crw-mcp/src/teardown.rs:36` calls
  `crw_renderer::browser::kill_all_browsers()` **unconditionally** (not feature-gated). Making
  `crw-renderer` optional will break the proxy build until this is gated — wrap the call (and any
  `crw_renderer` import in `teardown.rs`) in `#[cfg(feature = "embedded")]`, with a no-op fallback
  for the proxy build (no browsers to kill). This must land *with* the dep gating, not after.
  Verify proxy mode compiles & runs without `crw-renderer`/`crw-server`. (Confirmed: every other
  `crw_renderer`/`crw_server` reference in `crw-mcp/src/main.rs` — lines 40, 73, 86, 346, 427 —
  is already `#[cfg(feature="embedded")]`-gated; `teardown.rs:36` is the only ungated one.)
- **Note (out of scope but flagged):** `crates/crw-cli/src/teardown.rs:39` has the *same*
  unconditional `kill_all_browsers()` and `crw-cli` also hard-deps `crw-renderer`
  (`crw-cli/Cargo.toml:31`). This work targets a lean **`crw-mcp`** binary, not a lean `crw-cli`,
  so the cli teardown is left as-is — but record it so a future lean-cli effort knows the same
  gating is needed there.
- Add `opt-level = "z"` (or `"s"` — measure both) to a release profile; `lto`/`codegen-units`/
  `strip` are **already set** (`Cargo.toml:147-150`) — do not re-add. **No `panic = "abort"`** (D4).
- `tokio`/`reqwest` feature trimming is **cross-workspace surgery** (shared deps affect every
  crate) — scope it carefully or skip; keep `reqwest` multipart (`crw_parse_file` needs it).
  Replacing `reqwest` with a micro-client is over-engineering (marginal win, breaks multipart) —
  out of scope.
- Measure (platform-correct): binary size + `cargo bloat --release -p crw-mcp --crates`; idle
  RSS (macOS: `/usr/bin/time -l` or `ps -o rss`, **not** GNU `time -v`); cold-start to first
  `initialize` response via `hyperfine` (multi-run). Record proxy vs embedded numbers; target
  proxy ≤ ~6 MB / 10–20 MB RSS, embedded documented as the heavy path. No UPX (startup penalty).
- **Effort:** M (feature-gating may surface compile coupling). **Risk:** Low–Med.

### Phase 5 — First-class lean proxy build (no default flip)
**Files:** `crates/crw-mcp/Cargo.toml`, `crates/crw-mcp/src/main.rs`, CI release config, docs.
- Per D3, do **not** change `default = ["embedded"]`. Instead produce a second, clearly-named
  lean artifact (e.g. `crw-mcp` built `--no-default-features` → `crw-mcp-thin`, or a documented
  proxy-only release binary) so each audience gets a one-line install.
- Add an explicit `--no-browser` / env to skip auto-spawn in embedded mode for HTTP-only users
  (`main.rs:383-408` already skips when a renderer is pre-configured; make it intentional).
- (Deferred, documented) Any future move to proxy-first default goes through a `CRW_MODE`
  env-var deprecation window — not in this work.
- **Effort:** M. **Risk:** Low (additive; no behavior change for existing users).

### Phase 6 — Finish & verify distribution (most scaffolding already exists)
**Files:** `mcp/crw-mcp/*` (existing wrapper), `server.json` (existing), `.github/workflows/`,
`docs/mcp*`.
- **npm wrapper already implemented** (`mcp/crw-mcp/package.json` optionalDependencies +
  `bin/crw-mcp.js` env→platform-pkg→GitHub-download fallback). Work = **test in a clean env**,
  confirm each platform package publishes in `release.yml`, and decide on a win32 story (the
  launcher already falls back to download for win32). Do not "rebuild" it.
- **`server.json` already exists** (`io.github.us/crw`) and a `publish-mcp-registry` job runs.
  Work = add the **npm/stdio** transport entry (currently OCI-only) and satisfy current registry
  requirements: npm package `mcpName` match, MCPB `fileSha256`, package-type metadata. Keep this
  last (few clients query the registry yet).
- **`.mcpb` bundle:** only meaningful for the **lean** binary (bundling a 150 MB+ embedded build
  is absurd) — **gated on Phase 5**. Treat `.mcpb`/MCPB format maturity as a risk (sparse docs,
  Claude-Desktop-only); ship after the lean artifact exists, with a `user_config` `CRW_API_KEY`
  (keychain) entry.
- **macOS codesign/notarize is a real blocker, not a one-liner:** needs an Apple Developer
  account ($99/yr), `notarytool` creds in CI secrets, and Apple's review — none exist in
  `release.yml` today. **Flag it explicitly.** Interim: document `xattr -d com.apple.quarantine`
  in install steps until notarization lands; npm-wrapper installs bypass Gatekeeper anyway.
- Docs: copy-paste `claude mcp add --transport stdio crw -- npx -y @crw/mcp` (+ `--env
  CRW_API_KEY=…`), Cursor deeplink, Claude Desktop `.mcpb`, HTTP-remote URL form. Clear startup
  error when a required env/dependency is missing.
- **Effort:** M (mostly verification + signing setup). **Risk:** Med (Apple signing external).

## Verification
- **Token budget:** `crw-mcp-proto` test asserts serialized `tools/list` is under the ceiling
  using `tiktoken-rs` (or conservative `chars/3`). Record ~3,400 → ≤ ~1,000 in the PR.
- **Tool selection:** 20-prompt eval before/after Phase 1; no regressions in tool choice.
- **Output bounds:** unit tests for the shared truncation helper across all tool result shapes
  (scrape/parse markdown, map links, search results + inlined content, **crawl-status pages**),
  asserting markers + opt-out (`limit:0`=unbounded). Test minified text has no pretty whitespace.
- **Proxy parity:** test that bounds apply in proxy mode (post-processing) and `crw-cli`, not
  just embedded — i.e. a fat upstream response is still truncated locally.
- **structuredContent (D1):** keep/adapt T1–T6 (`lib.rs:599-776`); slimmed `outputSchema` still
  validates flat + grouped responses (T4).
- **Annotations:** test each tool advertises the exact annotation set (esp. `crw_crawl`
  `readOnlyHint:false`).
- **Protocol:** `cargo test -p crw-server --test mcp` stays green; add invalid-version-header
  and unknown-tool tests per Phase 3 decisions.
- **Build/size:** `cargo build --release -p crw-mcp` (embedded) and
  `--no-default-features` (proxy); capture binary size, idle RSS (macOS `/usr/bin/time -l`),
  cold-start (`hyperfine`). Confirm proxy build compiles without `crw-renderer`.
- **stdio framing smoke test (early — in Phase 2/3, not deferred to 6):** a test that drives the
  real stdio loop end-to-end (`initialize` → `tools/list` → `tools/call`) over a pipe, so a
  framing/newline bug is caught before distribution, not after publish. The full install smoke
  test (below) stays in Phase 6.
- **Install smoke test:** clean-env `npx -y @crw/mcp` → manual
  `initialize`/`tools/list`/`tools/call` over stdio; verify `claude mcp add` attaches; verify
  `.mcpb` installs in Claude Desktop (after Phase 5).
- Commands: `cargo test -p crw-mcp-proto`, `cargo test -p crw-server --test mcp`,
  `cargo clippy --all-targets`, `cargo build --release -p crw-mcp [--no-default-features]`.

## Versioning & migration
- Most changes are behavior-affecting but schema-additive → **minor** bumps. Use `feat:` commits
  (release-please owns the bump/changelog; do not use `chore:` for the default-cap change).
  Workspace is `0.15.2` (pre-1.0, so technically no SemVer guarantee — but treat it as if 1.0+).
- **`crw_scrape` default field projection is subtractive, not additive.** Dropping `rawHtml`
  (and `screenshot`/full `metadata`) from the default MCP result *removes* fields an existing
  client may read — for a post-1.0 contract this is a **major/breaking** change, not minor.
  Mitigation that keeps it minor: make the heavy fields **opt-in via the existing `formats`
  param** (a client that wants `rawHtml` already passes `formats:["rawHtml"]` and still gets it),
  and only fields *not requested via `formats`* are dropped — i.e. we stop returning rawHtml when
  it was never asked for. **Prerequisite:** verify (in Phase 2) whether today's `crw_scrape`
  emits `rawHtml` unconditionally or only when `formats` requests it. If unconditional, the
  default projection is breaking and must be called out in CHANGELOG + considered for a guarded
  rollout (e.g. an opt-out env for one release). Resolve before implementing the projection.
  (Narrowed by review: `rawHtml`/`html`/`links` are already `formats`-gated and appear only when
  requested, so dropping them by default is a no-op; the only genuinely-subtractive change is
  dropping the **always-serialized `metadata`/debug** fields — scope the breaking-change analysis
  to those.)
- CHANGELOG must call out: new default truncation (with the `limit:0`/`maxLength:0` opt-outs),
  the `crw_scrape` field projection, the lean `--no-default-features` build, and the
  unknown-tool (`-32602`) / header semantics changes.
- No `crw-core`/REST contract change (D2), so `crw-saas`, `langchain-crw`, and n8n REST consumers
  are unaffected; only MCP-client consumers see the intentional, documented output bounds.

## Remaining open questions
- **Q-empirical (gates D1 final step):** Does Claude Code count a tool result's `content` text
  toward context when `structuredContent` is also present? Resolved by the empirical check in
  Phase 1 (procedure specified under D1) before Phase 2's result changes.
- **Q-rawHtml (gates the `crw_scrape` projection):** Does today's `crw_scrape` emit `rawHtml`
  unconditionally or only when `formats` requests it? Answer in Phase 2 decides whether the
  default projection is minor (opt-in via `formats`) or breaking (see Versioning).
- **Q-registry/npm scope:** Confirm npm org/scope (`@crw`? `@fastcrw`?) and the real GitHub
  owner — `server.json:4` currently reads `io.github.us/crw` (a worktree placeholder username);
  the wrapper's package name and `server.json` `name`/`mcpName` must all agree on the real
  owner before publishing.
- **Q-default-mode (future):** Whether/when to make proxy-first the default (needs the Phase 4
  measurements + a view on what fraction of installs have a remote API/key). Deferred past D3.

## Order of execution & risk table
| Phase | Files | Effort | Risk | Order |
|---|---|---|---|---|
| 1 — Trim descriptions + outputSchema | `crw-mcp-proto/src/lib.rs` (+tests) | M | Med | 1 |
| 2 — Bound & compact all results (incl. crawl-status, proxy, cli, parse_file gap) | `lib.rs`, `routes/mcp.rs`, `main.rs`, `cli/.../mcp.rs` | L–XL | Med | 2 |
| 3 — Annotations + protocol conformance | `lib.rs`, `routes/mcp.rs` | S | Low | 3 |
| 4 — Feature-gate renderer (+ `teardown.rs:36` cfg-gate) + size profile | `crw-mcp/Cargo.toml`, `teardown.rs`, ws `Cargo.toml` | M | Low–Med | 4 |
| 5 — First-class lean proxy build | `crw-mcp/Cargo.toml`, `main.rs`, CI, docs | M | Low | 5 |
| 6 — Finish/verify distribution + signing | `mcp/`, `server.json`, CI, docs | M | Med | 6 |

Phases 1–3 are the context-footprint wins (highest leverage, mostly `crw-mcp-proto`). Phase 4
unlocks the lean binary (renderer gating is the lever). Phases 5–6 are additive distribution
work; the only external blocker is Apple signing (Phase 6).

## Iteration log
- **Iteration 0** (2026-06-13): initial plan from 5-agent parallel research (MCP design, token
  economics, runtime weight, install UX, Firecrawl survey + crw-mcp audit).
- **Iteration 1** (2026-06-13): 5 reviewer agents + Codex. Addressed criticals — resolved
  blocking decisions into D1–D5; corrected `crw_crawl` annotations (`readOnlyHint:false`) and
  the `destructiveHint`-default trap; reframed `structuredContent` as spec-recommended (fixed
  the misattribution); added `crw_check_crawl_status` to output bounding; mandated proxy-mode +
  `crw-cli` bound enforcement; moved truncation to the MCP layer (no `crw-core` changes);
  dropped `panic=abort` (PDF `catch_unwind` crash risk); identified non-optional `crw-renderer`
  as the real weight lever (feature-gate it); re-scoped Phase 6 (npm wrapper + `server.json`
  already exist); flagged Apple signing as a real blocker; fixed macOS measurement tooling;
  tightened the token target to ≤1,000 with a real tokenizer; added a tool-selection eval and a
  versioning section. Deferred lazy/tool-search exposure with explicit rationale. Rejected:
  removing input params (D5 — UX regression for introspecting consumers); replacing `reqwest`
  (over-engineering).
- **Iteration 2** (2026-06-13): 3 second-pass reviewers + Codex. Fixed the D1 "illegal
  combination" mischaracterization (illegal = declared `outputSchema` + non-conformant
  `structuredContent`); resolved the contradictory bound semantics (omitted = bounded default,
  `0` = unbounded opt-out); mandated stripping MCP-only args before proxy/REST forwarding;
  specified the exact `crw_scrape` output projection + reclassified dropping `rawHtml`-by-default
  as potentially **breaking** (mitigated via `formats` opt-in; gated on Q-rawHtml); added the
  **verified `teardown.rs:36` compile blocker** (unconditional `kill_all_browsers()`) as a hard
  Phase-4 prerequisite with feature wiring; added the **verified `crw-cli` `crw_parse_file`
  parity bug** (`commands/mcp.rs:190`) to Phase 2 + a shared-code extraction strategy; switched
  the token test to `tiktoken-rs` (dev-dep) with a floor+15% CI ceiling; **verified** the
  `initialize` response emits no `instructions` field (no hidden token cost); softened the
  protocol-version 400 to SHOULD; picked `-32602` for unknown tools; added `idempotentHint`
  justification; bumped Phase 2 to L–XL. No open criticals remain; the two remaining gating
  questions (Q-empirical, Q-rawHtml) are explicit measurements scheduled inside the phases.
- **Iteration 3** (2026-06-13): 3 confirmation reviewers + Codex → spec & feasibility consensus.
  Final fixes: resolved the residual Phase-2 bound-semantics contradiction with D2 (omitted =
  bounded default, `0` = unbounded); made the token-test fallback consistent (`ceil(chars/3)`) +
  flagged `tiktoken-rs` CI build; **narrowed Q-rawHtml** (rawHtml is already `formats`-gated — the
  real subtractive change is dropping always-serialized `metadata`); **added the embedded
  `crw_search` SearXNG footgun fix** (conditional advertisement / actionable error — a real
  frictionless-install gap) and the one-sentence description that preserves the backend hint;
  **acknowledged SSRF as enforced server-side** in both modes (proxy doesn't re-validate, by
  design); flagged the second `crw-cli/src/teardown.rs:39` compile blocker (out of scope for the
  lean *crw-mcp* binary, recorded for a future lean-cli); added a debug-log payload-size cap
  (stderr bloat) and an early stdio-framing smoke test; pinned the tool-selection eval for
  reproducibility; noted the `server.json` placeholder owner. Outstanding `-32601`-vs-`-32602`
  for unknown tools is a documented style choice, not a spec issue. **Consensus reached:** no
  open critical/warning items; the two gating questions are scheduled measurements, not unknowns.
