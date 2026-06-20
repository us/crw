# fastCRW × osaurus Integration — Implementation Plan

## Context

osaurus (github.com/osaurus-ai/osaurus) is a native macOS (Swift, Apple Silicon,
macOS 15.5+) AI-agent harness whose pitch is "own your AI, fully offline." Its
built-in web tools are three first-party plugins: `osaurus.search`,
`osaurus.fetch`, `osaurus.browser`. We want fastCRW (CRW) — a Rust-native,
auto-escalating (HTTP→LightPanda→Chrome) web backend with SearXNG-optimized
search and first-class MCP — to become the better web backend inside osaurus.

**Why now:** research confirmed from source
(`tools/search/Sources/OsaurusSearch/Plugin.swift`) that osaurus's *zero-config
default* search is **unofficial DuckDuckGo no-JS HTML scraping** (regex over
`result__a`/`result__snippet`), racing DDG+Brave+Bing HTML scrapers under a ~12s
budget, surfacing a `challenge_page` error under CAPTCHA load. That is fragile and
not truly "self-owned" — exactly the gap CRW's local-first, auto-escalating engine
fills. **The strongest honest wedge: "fixes your `challenge_page` failures with
robust auto-escalating JS render + non-scraping search."**

### Verified facts (with the correction pass from iteration 1)

- **osaurus plugin model:** a plugin is a macOS **`.dylib` (cdylib)** loaded
  **in-process via a C ABI** — *not* a subprocess/HTTP-server/WASM. Entry
  `osaurus_plugin_entry` (v1) / `osaurus_plugin_entry_v2(host)` (v2+); host API
  **v6** current (v1–v5 ABI-frozen). Manifest = **JSON embedded in the dylib**
  (`get_manifest()`): `plugin_id, name, version, description,
  capabilities.tools[]{id, description, parameters(JSON Schema), requirements,
  permission_policy}`. Plugin languages: **Swift (default) or Rust**
  (`osaurus tools create my-plugin [--language rust]`). Build via
  `scripts/build-tool.sh`; iterate via **`osaurus tools dev`** (watch+symlink into
  `~/Library/Application Support/Osaurus/Tools/`). **There is NO `osaurus tools
  install` verb** (iter-1 correction). Existing web plugins (`tools/fetch`,
  `tools/search`) do their HTTP **directly in Swift via URLSession with their own
  SSRF guard** — they do not proxy to any sidecar.
- **⚠️ Host `http_request` BLOCKS loopback (CONFIRMED, not "to verify"):**
  `docs/plugins/HOST_API.md:439` lists blocked targets — `127.0.0.0/8`, `::1`,
  RFC1918, and **IPv4-mapped IPv6** (`::ffff:127.0.0.1`). Pinned by
  `Tests/Plugin/SSRFTighteningTests.swift:44` (`blocksIPv4MappedLoopback`). The
  host `checkSSRF(url:)` takes **no `allowPrivate` opt-out** for plugins (unlike
  the fetch *tool's* own `checkSSRF(url:allowPrivate:)`). **⇒ A dylib cannot proxy
  to a local CRW server via the host `http_request` callback. Dead path.** The
  only loopback routes are (a) the dylib's **own** URLSession/reqwest (does the
  host App Sandbox permit plugin-initiated loopback? — the one genuine unknown),
  or (b) embedding CRW in-process (no network at all).
- **osaurus MCP client:** aggregates external MCP servers via "Remote MCP
  Providers," config `~/.osaurus/providers/mcp.json`: `{name, url, enabled,
  authType, transport(http|stdio), headers}` and for stdio `{command, args, env}`.
  On connect → `tools/list`, registers tools **namespaced `provider_toolname`**.
  **Additive only** — cannot shadow/replace built-ins or be the "default."
- **CRW invocation surface (v0.16.0, AGPL-3.0):**
  - `crw serve` → HTTP REST default **:3000** (`serve.rs`; `CRW_HOST`/`CRW_PORT`/
    `CRW_CONFIG`). **⚠️ `crw serve` does NOT auto-spawn or auto-download a browser**
    — it only connects to a pre-configured `[renderer.*].ws_url`; with none, logs
    "No CDP renderer active — JS rendering disabled" (`serve.rs:62-67,109-111`). So
    a managed `crw serve` child is **HTTP-only, no JS render** (iter-1 correction —
    kills old Phase 3-B).
  - `crw mcp` → **stdio JSON-RPC**, **Embedded** backend self-spawns LightPanda
    (`browser::spawn_all_headless()`, `mcp.rs:362`; auto-downloads ~66 MB to
    `~/.crw/lightpanda`, `browser.rs:204-265`; Docker only as last resort). This is
    the **only** path that gives zero-config JS render. Proxy backend via
    `--api-url`/`--api-key`.
  - Scrape `POST /v1/scrape`: `{url(required), formats:["markdown"|"html"|
    "rawHtml"|"plainText"|"links"|"json"|"summary"], renderJs?(serde camelCase),
    renderer?, waitFor?}`. **`renderer` enum valid variants: `auto|lightpanda|
    chrome|chrome_proxy|playwright|camoufox` — `"http"` is NOT valid** (iter-1
    correction). Envelope `{success, data:{markdown, html, metadata{statusCode}},
    error}` (`types.rs:613`).
  - **Search needs SearXNG** (`crw-search`, SearXNG-only). No `searxng_url` →
    error mapped to **HTTP 503 `search_disabled`** (`error.rs:49`, not 400). In
    **embedded MCP**, `crw_search` is **only advertised when `state.searxng.is_some()`**
    (`crw-cli/src/commands/mcp.rs:71-76`) — i.e. with no SearXNG the tool **does not appear at all** (no
    error reaches the agent). `crw setup --local` bootstraps SearXNG via Docker on
    :8080. **Search is NOT zero-external.**
  - **⚠️ Auth open by default AND `CRW_HOST` defaults to `0.0.0.0`** (`serve.rs:13`,
    `config.rs:477`; `[auth].api_keys` empty → no auth layer). A naively-started
    `crw serve` is **LAN-exposed and unauthenticated** — an SSRF-capable
    scrape/crawl server (can hit `169.254.169.254`, internal hosts). **Any sidecar
    design MUST force `127.0.0.1` bind + an API key.**
- **Bundling/licensing:** LightPanda = **AGPL-3.0**, Zig, single static
  `lightpanda-aarch64-macos` ~66 MB, CDP/WS :9222, **self-described Beta**. macOS:
  downloaded binaries get `com.apple.quarantine`; unsigned/ad-hoc → Gatekeeper
  blocks → each Mach-O needs Developer-ID-sign + hardened runtime, archive
  notarized + **stapled per-Mach-O**. Hardened runtime on a third-party Zig binary
  may need JIT/unsigned-mem entitlement exceptions. **AGPL process boundary
  confirmed** (FSF GPL FAQ: socket/fork-exec = separate programs) — but
  *distributing* the AGPL binaries triggers **AGPL §6 corresponding-source offer
  for EACH binary** (CRW *and* LightPanda), satisfied by shipping unmodified +
  a written source offer / repo link; **modifying obliges publishing mods**.
- **Registry/PR:** registry = one `plugins/<plugin_id>.json`, PR to
  `osaurus-ai/osaurus-tools` (`master`). `scripts/validate.py` requires
  `{plugin_id, versions[].artifacts[]{os:"macos", arch:"arm64", url, sha256,
  minisign}, public_keys.minisign}`; it verifies sha256+minisign but **does not
  inspect zip contents** (so extra binaries are mechanically allowed but the host
  only loads the `.dylib`). **The minisign pubkey is TOFU-immutable** — pick the
  CRW org's permanent key once. **CODEOWNERS gates `plugins/` to @tpae.**
  **⚠️ The registry has ZERO third-party plugins ever merged; the only outside PR
  (#162) is stalled.** ~159 merged PRs are all maintainer/bot version bumps.
  There is **no priority/default field** — when two search tools coexist, the
  agent picks via a **capability-search / preflight ranker** over tool
  `name`+`description` (tpae's recent commits), not prose alone.

### What we are NOT doing (scope guard)

- NOT editing any first-party plugin (`osaurus.search.json` etc.) — hostile,
  CODEOWNERS-blocked.
- NOT bundling browserless/stealth (SSPL) in any path. Chrome escalation uses the
  **user's own Chrome** (not bundled) — so no SSPL or Chromium-redistribution
  surface either.
- NOT claiming a CRW tool is the literal "default" web_search — osaurus has no such
  override. Adoption is won by the preflight ranker + recall benchmark, not a flag.
- NOT shipping the "beats osaurus.fetch on recall" claim in any PR/README **before**
  the benchmark exists (iter-1: it is currently aspirational).
- NOT shipping `crw_search` in the first native release (SearXNG/Docker dependency
  makes the "offline" claim dishonest until setup is polished).

## Approach

Two tracks. **Track A ships now with zero Swift and is also the fallback.** Track B
(native registry plugin) is **gated on a Phase-0 transport decision AND on
maintainer interest** — it is discussion-gated, not scheduled.

1. **Track A — register `crw mcp` as a Remote MCP Provider (this week):** stdio,
   Embedded backend (self-spawns LightPanda → zero-config JS scrape offline). Tools
   surface as `crw_scrape`, `crw_map`, `crw_crawl` (and `crw_search` **only if** a
   SearXNG is configured). One idempotent setup script + a doc. This is the
   lazy-correct first rung and the permanent fallback if Track B is blocked.
2. **Track B — native v6 plugin (`crw.web`), registry-listed — only if it clears
   two gates:**
   - **Gate 1 (Phase 0): a viable transport.** The host-callback proxy is dead
     (loopback blocked). Phase 0 chooses among three real options (below).
   - **Gate 2: maintainer tolerance.** Open a discussion/issue with @tpae *before*
     building, covering: (a) receptiveness to a third-party web plugin overlapping
     core, (b) tolerance for an **AGPL** plugin in the registry. If no engagement
     in ~1 week → **stop at Track A** (do not sink the notarization budget).
3. **Security is a hard contract for any sidecar:** bind `127.0.0.1` only, random
   free port, generate + require an API key, the plugin refuses any non-loopback
   CRW endpoint, redact keys in logs. Exposing CRW on LAN is unsupported.
4. **Lead with scrape/fetch/render; search is phase-2** (SearXNG gap).

### The three Track-B transports (Phase 0 decides)

| # | Transport | Pros | Cons / unknowns |
|---|---|---|---|
| **T1** | Thin **Swift** dylib → its **own URLSession** → `127.0.0.1` locked-down `crw serve` (separate process) | Cleanest AGPL boundary (separate process); reuses CRW unchanged | **Unknown: does osaurus sandbox plugin-initiated loopback?** Needs a managed/locked-down server + lifecycle. `crw serve` has no JS render unless a renderer is provisioned (see note) |
| **T2** | **Rust** cdylib **embedding `crw-core`/`crw-renderer` in-process** | No HTTP server, no loopback, no open-auth/SSRF-server surface, no port | **Still provisions LightPanda** (download/quarantine/sign + child-process reaping) — "no lifecycle" applies only to the HTTP server, not the browser; plugin becomes **AGPL** (full corresponding source); tokio-runtime ownership inside host; C-ABI export; binary size; **registry tolerance of AGPL plugin unknown** |
| **T3** | Plugin manages a **`crw mcp` stdio child** and speaks MCP to it (gets auto-spawn LightPanda) | Real JS render with zero browser provisioning | Largely duplicates Track A; native value reduces to "registry-listed + better tool descriptions" |

**Working recommendation (revisit after Phase 0):** **T2 (embedded Rust cdylib)**
is architecturally cleanest and erases the entire sidecar/security/lifecycle tree
— *if* AGPL-plugin registry tolerance (Gate 2) and in-host async-runtime feasibility
check out. Else **T1** with a hardened locked-down server. T3 is a strict superset
of Track A and only worth it for the registry listing.

> Renderer note for T1: a plugin-spawned `crw serve` renders JS **only** if it also
> provisions a CDP target. Cheapest: have the plugin run `crw mcp`-equivalent setup
> first (downloads LightPanda to `~/.crw`) and point `crw serve` at the resulting
> `ws_url`, or spawn the child as `crw mcp` (T3). Do not assume `crw serve` renders.

### Trade-offs accepted

- **Track B is conditional, not committed.** Given a registry with zero
  third-party merges, building a full notarized native plugin before maintainer
  signal is a bad bet (iter-1 GTM + Codex). Track A captures the value now.
- **T2's AGPL plugin vs T1's clean boundary** is a genuine fork resolved by Gate 2,
  not by us guessing.
- **Search deferred** — honest "offline" lead requires it.

## Phases

### Phase 0 — Source audit + transport decision (replaces blind spikes)

Most prior unknowns are answered by reading public source, not throwaway plugins
(iter-1: spike-first was wasteful). Do, in order:

- **0a — confirm loopback block (DONE in review):** host `http_request` blocks
  loopback. Conclusion recorded; no spike needed. **T1 viability is broader than
  "URLSession reaches loopback"** — the 0a spike must exercise the *full* T1 path:
  (i) plugin's own `URLSession`-GETs `http://127.0.0.1:<port>/health`; (ii) plugin
  **launches `crw serve`** with a temp locked-down config (127.0.0.1 + generated
  key); (iii) **clean shutdown** on `destroy`. Pass on all three ⇒ T1 viable.
  *Note:* osaurus loads user-dropped dylibs with "no code signing required" and
  runs *agent* code (not the host) in a Containerization VM — strong evidence the
  **host app is hardened-runtime-only, not App-Sandboxed**, so loopback is *likely*
  permitted and 0a's network sub-question is lower-risk than it looks. Still verify
  the launch+cleanup parts. *Effort: 0.5–1 day.*
- **0b — Rust cdylib feasibility (T2):** confirm `osaurus tools create --language
  rust` produces a loadable cdylib; assess (i) exporting the C ABI
  (`osaurus_plugin_entry_v2`) from a crate that also pulls `crw-core`, (ii)
  **tokio runtime ownership** — `crw-core`/`crw-renderer` are tokio *libraries*
  (no `#[tokio::main]`; that lives in the binaries' `main.rs`), so a cdylib can own
  its own `Runtime` and `block_on`/spawn — verify it does not block the host;
  (iii) cdylib binary size with crw-core linked; (iv) browser provisioning when
  embedded (does it reuse `spawn_all_headless` cleanly?). **Two T2-specific hazards
  the spike MUST exercise, not just `load + get_manifest()`:** (a) **child-process
  reaping** — `spawn_all_headless` spawns LightPanda as a child, but inside a
  foreign Swift host the dylib does not own SIGCHLD disposition (libdispatch may
  reap, or the child may zombie); (b) **process-global `OnceLock` statics**
  (`metrics.rs`, `preference.rs` PSL, health telemetry) re-initialized after a
  `dlclose`/reload (osaurus `tools dev` watch+symlink) are UB. Test reload + a real
  render. *Effort: 1–2 days.*
- **0c — Gate 2 maintainer discussion:** open an issue/Discord thread with @tpae:
  third-party web plugin tolerance + AGPL-plugin tolerance. *Async; start Day 1.*
- **Decision:** pick T1 / T2 / (fallback) Track-A-only based on 0a, 0b, 0c.
- Effort: 2–3 days wall-clock (0c overlaps). Risk: 0a LOW · 0b MED · 0c HIGH(gate).

### Phase 1 — Track A: MCP registration (ships independently, FIRST)

Validate this before any native work — it is the fallback and needs the same
clean-machine setup (iter-1/Codex: don't parallelize with native spikes if they
contend for the same fresh Mac).

- Files: `crw-opencore/docs/integrations/osaurus.md`; `scripts/osaurus-register.sh`
  (idempotent jq merge into `~/.osaurus/providers/mcp.json`, backs up first).
- Entry (schema verified against osaurus source):
  ```json
  { "name": "crw", "enabled": true, "transport": "stdio",
    "command": "crw", "args": ["mcp"], "env": {} }
  ```
- Steps: (1) confirm `crw mcp` Embedded starts clean over stdio, no config;
  (2) confirm LightPanda auto-download/spawn on a fresh Mac with **no Docker**;
  (3) document that `crw_search` **will not appear** unless SearXNG is configured
  (not an error — the tool is hidden); (4) doc + script + a 60-sec capture.
- Effort: 1 day. Risk: LOW. **Fallback-complete: if Track B is blocked, this is the
  shipped product.**

### Phase 2 — Track B scaffold (only after Phase 0 picks T1/T2)

- `osaurus tools create crw-web [--language rust]` → `tools/crw.web/`.
- Manifest `plugin_id: "crw.web"`, minimal tools first:
  - `crw_scrape` — `{url(required), formats?, renderJs?, renderer?}` (valid
    renderer enum only), `requirements:["network"]`.
  - `crw_fetch` — markdown convenience (reads as a drop-in for `osaurus.fetch`).
- `invoke`: build body → reach CRW via the chosen transport → return
  `data.markdown`/`data`. **Per-transport response contract (define before coding;
  the envelope is NOT uniform):** T1 returns the HTTP `{success,data,error}` JSON
  envelope; **T2** returns typed Rust values in-process (no HTTP envelope to
  unwrap); **T3** returns MCP `content[]` JSON. Fixture tests must match the chosen
  transport's shape. Error mapping must branch on **503 `search_disabled`** (T1) /
  the equivalent typed/MCP error and server-down → actionable text.
- Tool descriptions written for the **preflight ranker** (iter-1), not just humans:
  "auto-renders JS/SPA/Cloudflare pages, higher recall than HTTP-only fetch, local."
- Effort: T2 2–4 days (runtime/ABI) · T1 2–3 days. Risk: MED.

### Phase 3 — Track B: lifecycle & security (T1 only; T2 has none)

If **T2** chosen: skip — no sidecar, no server, no lifecycle. (This is T2's whole
point.)

If **T1** chosen, the server contract is non-negotiable:
- Spawn/locate `crw serve` bound to **`127.0.0.1` + random free port** with a
  **generated API key** (`CRW_HOST=127.0.0.1`, `[auth].api_keys=[<key>]`); plugin
  sends `Authorization: Bearer`. Plugin **refuses** any non-loopback endpoint.
- Provision a renderer (point at LightPanda in `~/.crw`, or run the child as
  `crw mcp`) — do **not** assume `crw serve` renders JS.
- Supervise: health-check, restart-on-crash, shutdown on `destroy`, redact keys in
  logs. Bundling the CRW binary ⇒ sign+notarize (Phase 5).
- Effort: 3–5 days (supervision + signing is the real cost; iter-1 raised this).
  Risk: MED.

### Phase 4 — search tool (SECOND registry release, after the scrape/fetch PR)

**Ordering note:** the *first* native registry PR (Phase 5) ships **scrape/fetch
only**; `crw_search` lands in a **subsequent release/PR** once SearXNG setup is
polished. So the real execution order is Phase 2 → 3 → **5 (scrape/fetch PR)** →
Phase 4 → a second packaging+PR. Phase 4 is numbered before 5 only because it is
defined here; it *executes* after the first PR.

- Add `crw_search` only after scrape/fetch lands AND SearXNG setup is polished.
  `{query(required), limit?, lang?}` → `/v1/search`.
- On 503 `search_disabled`: actionable error pointing at `crw setup --local`
  (Docker SearXNG :8080). **Never** silently fall back to DDG scraping (that just
  reinvents osaurus's fragile default).
- Decide + document: self-hosted SearXNG (offline) vs managed `api.fastcrw.com`
  (NOT offline — must be labeled as such wherever search is documented).
- Effort: 1–2 days. Risk: MED (SearXNG/Docker is the weakest UX link).

### Phase 5 — packaging, signing, registry PR (only if Gate 2 passed)

- Build via `scripts/build-tool.sh` → signed dylib zip (+ SKILL.md, README,
  CHANGELOG). T1-bundled / any extra Mach-O ⇒ Developer-ID-sign each binary
  bottom-up + hardened runtime, notarize the archive, then **unzip → staple each
  Mach-O → re-zip**. Verify with `spctl -a -vvv` AND a *running* launch on a fresh
  Mac (static check ≠ runtime acceptance).
- `plugins/crw.web.json`: artifacts `{os:"macos", arch:"arm64", url, sha256,
  minisign}` + `public_keys.minisign` (**permanent TOFU key — choose once**). Host
  the zip on a CRW GitHub release.
- **First PR ships scrape/fetch only.** Framing — lead with "robust auto-escalating
  JS render that fixes HTTP-only `osaurus.fetch`'s SPA/Cloudflare misses." Save the
  "non-scraping local search" pitch for the Phase-4 search release (don't promise
  search in the scrape/fetch PR). Do **not** lead with "Firecrawl-compatible." Note
  AGPL + source availability. **Only open after Gate-2 engagement.**
- Effort: 2–4 days (notarization is the time sink; Developer-ID cert ownership =
  Open Q). Risk: MED–HIGH.

## Verification

- **Phase 0a:** Swift plugin URLSession→`127.0.0.1/health` — binary pass/fail.
  **0b:** a Rust cdylib that links crw-core loads in osaurus and returns a
  `get_manifest()` — pass/fail + measured binary size.
- **Phase 1 (Track A):** clean Mac, register → agent scrapes a JS SPA + a static
  page; `crw_scrape` returns rendered markdown; confirm `crw_search` is absent
  without SearXNG and present after `crw setup --local`.
- **Phase 2–4 — deterministic fixtures, not "drive a chat and eyeball"** (iter-1/
  Codex): unit-test `invoke` envelope-unwrapping against recorded CRW JSON for:
  static HTML, SPA-needs-JS, anti-bot/thin-body escalation, timeout, malformed CRW
  response, server-down, **503 `search_disabled`**, non-200 statusCode, large-body
  truncation, concurrent tool calls. Plus a **preflight-ranker eval**: does osaurus
  pick `crw_scrape`/`crw_fetch` over built-ins across ≥10 realistic user phrasings?
- **Adoption benchmark (also the PR evidence):** CRW vs `osaurus.fetch` recall on
  10 JS-heavy URLs. Must exist **before** any recall claim ships.
- **Phase 5:** `validate.py plugins/crw.web.json` passes; `spctl -a -vvv` accepted;
  fresh-Mac registry install launches without Gatekeeper prompts.

## Open questions

1. **0a:** does osaurus's App Sandbox permit a plugin's own URLSession to reach
   `127.0.0.1`? (Determines T1 viability.)
2. **0b:** can a Rust cdylib embed crw-core and own a tokio runtime inside the Swift
   host without blocking it? Binary size acceptable? (Determines T2.)
3. **Gate 2:** will @tpae accept (a) a third-party web plugin overlapping core and
   (b) an **AGPL-licensed** plugin in the registry? (Determines whether Phase 2–5
   happen at all.)
4. **Search UX:** Docker-SearXNG (offline) vs managed `api.fastcrw.com` (not
   offline) — which do we recommend, and does the latter contradict the pitch?
5. **Notarization ownership:** who holds the Developer ID cert / runs notarize CI
   for the CRW-distributed zip? ($99/yr Apple account — real blocker, iter-1.)
6. **T2 AGPL combination:** does an in-process AGPL cdylib loaded by osaurus create
   any obligation on osaurus itself, or is the runtime-combination purely the
   user's? (Get a clear read before T2; affects Gate 2.)

## Order of execution & risk table

| Phase | Files | Effort | Risk | Order |
|---|---|---|---|---|
| 1 — Track A MCP registration | `docs/integrations/osaurus.md`, `scripts/osaurus-register.sh` | 1 day | LOW | **1st (ship + fallback)** |
| 0 — Source audit + transport decision (0a/0b/0c) | 1 T1 spike plugin (launch+loopback+cleanup) + 1 T2 cdylib spike; @tpae thread | 2–3 days | 0a LOW·0b MED·0c HIGH | 2nd (0c starts Day 1) |
| 2 — Track B scaffold (scrape/fetch) | `tools/crw.web/**` | 2–4 days | MED | 3rd (if Phase 0 green) |
| 3 — T1 lifecycle+security (skip if T2) | `Plugin.swift`, supervisor | 3–5 days | MED | 4th (T1 only) |
| 5 — packaging + notarize + **scrape/fetch** registry PR | `plugins/crw.web.json`, notarize CI | 2–4 days | MED–HIGH | 5th — **first PR** (if Gate 2 passed) |
| 4 — search tool + **second** PR | `Plugin.swift`, manifest | 1–2 days | MED | last (after first PR lands) |

**Bottom line:** Ship Track A now. Run Phase 0 (incl. the @tpae conversation) in
parallel. Only commit Track B once a transport (T1/T2) is proven *and* maintainers
signal they'll accept an AGPL third-party web plugin — otherwise Track A + a
"works with osaurus" blog/benchmark is the honest, lower-cost win that also reaches
every other MCP harness.

## Iteration log
- **Iteration 0** (2026-06-20): initial plan from 5 parallel research agents.
- **Iteration 1** (2026-06-20): 5 reviewers + Codex. Addressed 4 critical consensus
  issues — (1) host `http_request` loopback block confirmed ⇒ host-callback proxy
  removed, three explicit transports (T1 URLSession / T2 embedded Rust cdylib / T3
  managed `crw mcp`) introduced with T2 elevated to first-class; (2) `crw serve`
  does not auto-spawn a browser ⇒ old Phase 3-B rewritten; (3) CRW `0.0.0.0`+open-
  auth ⇒ hard 127.0.0.1+API-key security contract added; (4) registry has zero
  third-party merges ⇒ Track B made discussion-gated (Gate 2) with Track A as the
  shipped fallback. Fixed factual errors: CLI verbs (`tools create`/`tools dev`, no
  `install`), search 503-not-400 + hidden-when-no-SearXNG, invalid `renderer:"http"`,
  AGPL §6 source-offer for both binaries, TOFU-immutable minisign key, notarization
  stapling/cost, preflight-ranker (not prose) as the selection mechanism. Deferred
  `crw_search` out of first native release; recall claim gated on a benchmark.
  Rejected nothing — all findings actioned.
- **Iteration 2** (2026-06-20): 3 reviewers + Codex; 2 reviewers + Codex's only
  remaining items were warnings (no criticals). Addressed all: (1) Phase 4/5
  ordering contradiction — first registry PR ships scrape/fetch only, search is a
  second release/PR (clarified in Phase 4 note, Phase 5 framing, order table);
  (2) T2 "no lifecycle" qualified — still provisions LightPanda (download/quarantine/
  sign + child-process reaping); (3) Phase 0a broadened to test the full T1 path
  (launch + loopback + cleanup), with a note that the host is likely
  hardened-runtime-only (not App-Sandboxed) so loopback is lower-risk; (4) Phase 2
  per-transport response contract added (T1 HTTP envelope vs T2 typed vs T3 MCP
  content); (5) 0b now names the two T2 hazards to exercise (SIGCHLD child-reaping,
  OnceLock-on-reload UB); fixed `crw-cli/src/commands/mcp.rs` citation and the
  risk-table spike-label. Two reviewers + the CRW-fact reviewer returned
  [CONSENSUS]; remaining warnings resolved here. Plan considered consensus-ready.
