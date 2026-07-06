use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub renderer: RendererConfig,
    #[serde(default)]
    pub crawler: CrawlerConfig,
    #[serde(default)]
    pub extraction: ExtractionConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub request: RequestConfig,
    #[serde(default)]
    pub search: SearchConfig,
    #[serde(default)]
    pub map: MapConfig,
    /// `[document]` — binary-document (PDF) parsing knobs.
    #[serde(default)]
    pub document: DocumentConfig,
    /// `[client]` — settings for the local CLI/MCP when it proxies to the
    /// hosted SaaS. Written by `crw setup` into the user-config file.
    #[serde(default)]
    pub client: ClientConfig,
}

/// `[client]` — cloud-proxy credentials populated by `crw setup` and read by
/// `crw mcp` / `crw-mcp`. Both fields are `Option` so an unconfigured user runs
/// in local mode without surprise overrides.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ClientConfig {
    /// Base URL of the hosted CRW API, e.g. `https://api.fastcrw.com`.
    #[serde(default)]
    pub api_url: Option<String>,
    /// API key for the hosted CRW API.
    #[serde(default)]
    pub api_key: Option<String>,
}

/// `[document]` section — controls PDF (and future binary-document) parsing.
/// All knobs honor `CRW_DOCUMENT__*` env overrides.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct DocumentConfig {
    /// Master switch for document parsing at runtime (independent of the
    /// compile-time `pdf` cargo feature). When `false`, PDFs are left unparsed.
    pub enabled: bool,
    /// Cap on the number of pages converted per document. `0` = no limit.
    pub max_pages: usize,
    /// Best-effort extraction from scanned/image PDFs (no OCR; usually empty).
    pub attempt_scanned: bool,
    /// Maximum upload size in bytes for `POST /v2/parse`. Defaults to 50 MB,
    /// matching the HTTP renderer's response cap.
    pub max_upload_bytes: usize,
    /// Maximum number of concurrent uploads being parsed at once — bounds peak
    /// memory (each in-flight upload buffers up to `max_upload_bytes`).
    pub upload_concurrency: usize,
    /// Process-wide cap on concurrent PDF parses across ALL surfaces (URL
    /// scrape, crawl, batch, upload). Bounds peak CPU + decompressed memory: a
    /// malicious PDF can decompress far beyond its on-wire size, so this is the
    /// primary memory-DoS guard. Independent of `upload_concurrency` (which
    /// only bounds upload body buffering).
    pub max_concurrent_parses: usize,
    /// Wall-clock timeout (ms) for a single PDF parse. A parse exceeding this
    /// returns a timeout error to the caller; protects against pathological
    /// documents that spin the parser. `0` disables the timeout.
    pub parse_timeout_ms: u64,
    /// Decompression-bomb guard: maximum total DECOMPRESSED bytes a document's
    /// FlateDecode streams may inflate to. Checked in bounded memory BEFORE the
    /// parser runs, so a small file that explodes to many GB is rejected with
    /// `pdf_too_large` having allocated only kilobytes. This is the primary
    /// guard against OOM-crashing the host. `0` disables it. Default 100 MiB —
    /// huge for text extraction (millions of words) yet tiny next to host RAM.
    /// Raise only if you must parse image-heavy PDFs.
    pub max_decompressed_bytes: usize,
    /// Run each PDF parse in an isolated child PROCESS (Unix only) instead of
    /// in-process. The child gets a hard OS memory ceiling (`RLIMIT_AS`) and CPU
    /// limit, inherits no env/secrets, and is killed on timeout. A crash, OOM,
    /// or even a hypothetical parser RCE is contained to the child — the main
    /// server (scrape/crawl) keeps running. Costs ~1-3ms spawn overhead per
    /// parse. Recommended for hosts that accept untrusted uploads. Default off.
    pub sandbox: bool,
    /// Hard address-space limit (bytes) for a sandbox child (`RLIMIT_AS`). The
    /// child is aborted by the OS if it allocates beyond this — the ultimate
    /// backstop against memory-DoS even if the decompression guard is bypassed.
    /// Default 512 MiB.
    pub sandbox_memory_bytes: u64,
}

impl Default for DocumentConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_pages: 0,
            attempt_scanned: false,
            max_upload_bytes: 52_428_800, // 50 MiB
            upload_concurrency: 4,
            max_concurrent_parses: 4,
            parse_timeout_ms: 30_000,
            max_decompressed_bytes: 104_857_600, // 100 MiB
            sandbox: false,
            sandbox_memory_bytes: 536_870_912, // 512 MiB
        }
    }
}

/// `[map]` section — currently only carries `[map.url_filter]`.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct MapConfig {
    #[serde(default)]
    pub url_filter: MapUrlFilterConfig,
}

/// `[map.url_filter]` — raw TOML view of the filter knobs. Conversion to
/// the runtime `UrlFilterCfg` lives in `crw-crawl` (which can see both this
/// type and the filter module). Keeping this struct dependency-free here
/// avoids a cycle (`crw-core` does not depend on `crw-crawl`).
#[derive(Debug, Clone, Deserialize)]
pub struct MapUrlFilterConfig {
    /// Tier B — strip tracking params. Default: `true`.
    #[serde(default = "default_true_filter")]
    pub strip_tracking_params: bool,
    /// Tier A — drop action URLs entirely. Default: `true`.
    #[serde(default = "default_true_filter")]
    pub drop_action_urls: bool,
    /// When `true`, `.gov`/`.mil` hosts run Tier A too. Default `false`.
    #[serde(default)]
    pub gov_tld_drop_actions: bool,
    /// Additive on top of `DEFAULT_TRACKING_PARAMS`. Keys are normalized to
    /// canonical form (lowercase, `-` folded to `_`).
    #[serde(default)]
    pub extra_tracking_params: Vec<String>,
    /// Additive on top of `DEFAULT_ACTION_PARAMS`. Keys are normalized to
    /// canonical form (lowercase, `-` folded to `_`).
    #[serde(default)]
    pub extra_action_params: Vec<String>,
    /// Additive on top of `ALWAYS_PRESERVE`. Keys are normalized to
    /// canonical form (lowercase, `-` folded to `_`).
    #[serde(default)]
    pub extra_preserve_params: Vec<String>,
}

impl Default for MapUrlFilterConfig {
    fn default() -> Self {
        Self {
            strip_tracking_params: true,
            drop_action_urls: true,
            gov_tld_drop_actions: false,
            extra_tracking_params: Vec::new(),
            extra_action_params: Vec::new(),
            extra_preserve_params: Vec::new(),
        }
    }
}

fn default_true_filter() -> bool {
    true
}

/// Per-tier CDP overhead in milliseconds — sum of SPA selector poll budget,
/// challenge retry budget, content-stability budget, and fetch overhead.
/// Mirrors the constants in `crw-renderer::cdp`. The drift between the two
/// is regression-tested by `crates/crw-server/tests/cdp_constants_test.rs`
/// (gated behind `feature = "cdp"`).
///
/// Used by [`RendererConfig::min_deadline_for_full_ladder_ms`] so the request
/// deadline accommodates each CDP tier's outer fetch timeout, not just its
/// configured `page_timeout`.
pub const CDP_TIER_OVERHEAD_MS: u64 = 28_000;

/// Hard upper bound on the per-request `wait_for_ms` budget. The Tower outer
/// timeout is sized so a worst-case implicit scrape (no `deadlineMs`,
/// `wait_for` at this maximum) still completes inside it; values above this
/// are clamped by [`AppConfig::effective_deadline_ms`] so the inner deadline
/// can never escape the outer envelope. Documented as `(0, 60000]` in
/// `types.rs::ScrapeRequest::wait_for`.
pub const MAX_WAIT_FOR_MS: u64 = 60_000;

/// Default Camoufox REST per-request budget (ms). Covers the full REST
/// round-trip (create tab → evaluate `outerHTML` → destroy session) plus
/// Camoufox's anti-bot navigation. 60s mirrors the established camofox-browser
/// client navigate budget. Used by [`RendererConfig::camoufox_timeout`].
///
/// There is intentionally no `CAMOUFOX_*_OVERHEAD_MS` analogue to
/// [`CDP_TIER_OVERHEAD_MS`]: Camoufox is a single REST tier, not a CDP tier,
/// and must never be charged CDP overhead.
pub const CAMOUFOX_DEFAULT_TIMEOUT_MS: u64 = 60_000;

/// Configuration for the `/v1/search` endpoint and its SearXNG backend.
///
/// When `searxng_url` is unset the endpoint returns HTTP 503 with
/// `error_code: "search_disabled"` — the route remains mounted so that
/// startup doesn't have to know whether search will ever be configured.
#[derive(Debug, Clone, Deserialize)]
pub struct SearchConfig {
    /// Master switch. Defaults to `true`; set to `false` to refuse all
    /// `/v1/search` requests even if `searxng_url` is configured.
    #[serde(default = "default_true_search")]
    pub enabled: bool,
    /// Base URL of the SearXNG instance (e.g. `http://searxng:8080`).
    /// `None` (the default) disables the endpoint with a clear error.
    #[serde(default)]
    pub searxng_url: Option<String>,
    /// OpenAlex API key for the `/v1/search/research/*` endpoints (CC0 data,
    /// commercial use OK). `None` falls back to the keyless polite pool.
    #[serde(default)]
    pub openalex_api_key: Option<String>,
    /// Contact email for OpenAlex's "polite pool" (`mailto=`), recommended for
    /// higher rate limits. `None` omits it.
    #[serde(default)]
    pub openalex_mailto: Option<String>,
    /// Semantic Scholar API key (`x-api-key`) for the research endpoints'
    /// full-text snippet + citation-graph boosters. `None` uses the shared
    /// (1 RPS) unauthenticated tier.
    #[serde(default)]
    pub s2_api_key: Option<String>,
    /// End-to-end timeout for the SearXNG call in milliseconds.
    #[serde(default = "default_search_timeout_ms")]
    pub timeout_ms: u64,
    /// Default `limit` when the request omits it.
    #[serde(default = "default_search_limit")]
    pub default_limit: u32,
    /// Hard cap on `limit` per request. SaaS uses 20.
    #[serde(default = "default_search_max_limit")]
    pub max_limit: u32,
    /// SearXNG engines invoked when the request includes `categories: ["research"]`.
    /// Defaults match the SaaS implementation.
    #[serde(default = "default_research_engines")]
    pub research_engines: Vec<String>,
    /// SearXNG engines invoked when the request includes `categories: ["github"]`.
    #[serde(default = "default_github_engines")]
    pub github_engines: Vec<String>,
    /// Re-rank the flat result pool for the LLM answer / summarize path
    /// (RRF + junk/coverage/geo filter + BM25 + domain dedupe) instead of the
    /// raw SearXNG-score sort. Defaults to `true`. The plain (non-LLM) path is
    /// unaffected and keeps SaaS byte-parity regardless of this flag.
    #[serde(default = "default_true_search")]
    pub rerank_enabled: bool,
    /// Multi-query expansion for the LLM answer / summarize path: before the
    /// SearXNG fetch, generate an entity/keyword-focused rewrite of the query,
    /// fetch both the original and the rewrite, and UNION the candidate pools
    /// (recall can only increase — the original's results are always kept).
    /// Targets "retrieval-miss" failures where the answer's source never
    /// surfaced for the user's phrasing. Costs one extra small LLM call + one
    /// extra SearXNG fetch. Defaults to `false` (gated); the plain path and the
    /// answer layer are untouched, so precision/SaaS-parity are preserved.
    #[serde(default)]
    pub query_expand: bool,
    /// Number of LLM-generated query rewrites to fetch + union when
    /// `query_expand` is on. `1` reproduces the original single-variant
    /// behavior. Higher values request more DIVERSE reformulations
    /// (abbreviation/acronym-expanded, keyword-focused) and fetch their pools
    /// in parallel, raising recall on retrieval-miss queries (e.g. an
    /// unexpanded acronym whose page never surfaced) at the cost of one extra
    /// SearXNG fetch each. Clamped to `MAX_QUERY_EXPAND_VARIANTS` in the route.
    #[serde(default = "default_query_expand_variants")]
    pub query_expand_variants: usize,
    /// Phase C1 (latency-qn): on the answer path with query_expand + scrapeOptions,
    /// scrape the original-query results CONCURRENTLY with the expansion (LLM
    /// rewrite + variant SearXNG fetches), then union and reuse the scrapes.
    /// Final source set is identical to the serial path (rerank over the same
    /// union) → quality-neutral; only the scheduling overlaps the ~5-10s
    /// expansion overhead. Default off.
    #[serde(default)]
    pub pipeline_overlap: bool,
    /// Adaptive multi-round retrieval (the "evidence-scout" loop). When the
    /// round-1 answer ABSTAINS (sources lacked the fact), an LLM scout reads the
    /// round-1 evidence and emits targeted follow-up queries (acronym-expanded,
    /// exact-entity, predicate/date-specific); their results are scraped, unioned
    /// into the pool, and the answer is re-synthesized ONCE. Bounded (one extra
    /// round, capped follow-up queries) so worst-case stays within the request
    /// deadline. Only fires on abstention, so ~most queries keep the single-shot
    /// fast path. Recall-only + monotone-safe: a still-abstaining round-2 is
    /// discarded, keeping round-1. Targets "the answer page never entered the
    /// first pool" — the dominant remaining miss. Defaults to `false` (gated).
    #[serde(default)]
    pub multi_round: bool,
    /// Passage-level relevance gate for the LLM answer path: split each scraped
    /// source into passages and feed the answer LLM only the query-relevant
    /// ones (DeepSeek-scored, no new ML deps). Subtractive — removes noise, never
    /// adds sources or forces commits; falls back to the full source on any
    /// failure (byte-identical to off), so it is monotone-safe. Defaults to
    /// `false` (gated); answer prompt + plain path untouched.
    #[serde(default)]
    pub passage_select: bool,
    /// Page-2 fallback for the LLM answer / summarize path: if the reranked
    /// (junk-filtered, deduped) candidate pool comes back thinner than the
    /// answer needs (`< answer_top_n`), fetch the SAME query's SearXNG page 2
    /// once and union it in, then re-rank. The trigger is evaluated POST-rerank,
    /// so a junk-heavy first page does not suppress it; the extra fetch only
    /// fires on already-under-yielding queries (QPS never doubles across the
    /// corpus). Recall-only + abstention is untouched (a sparse page1+page2 pool
    /// still abstains). Defaults to `false` (gated); requires `rerank_enabled`.
    #[serde(default)]
    pub page2_fallback: bool,
    /// Calibrated answer path (gated): reduce recoverable OVER-abstentions by
    /// (a) feeding more sources to the answer LLM by default (top_n 5->8, so the
    /// answer in result #6-8 or behind a failed top-5 scrape still reaches it)
    /// and (b) swapping the answer prompt's abstention rule for an anti-hedge
    /// variant — commit when the sources DO contain the answer (even indirectly
    /// / one inference step), abstain ONLY when they genuinely lack it. The
    /// "use ONLY sources" grounding is untouched, so this is the precise inverse
    /// of the cycle-1 blunt "always commit" failure (which forced commits on
    /// no-source cases). Default false; A/B with an INCORRECT-guard before flip.
    #[serde(default)]
    pub answer_calibrated: bool,
    /// Moat-hardening abstention (gated). Appends a clause making the answer
    /// model (a) REJECT a false/unverifiable premise instead of answering as
    /// though it were true, (b) report when sources CONFLICT rather than picking
    /// one confidently, and (c) abstain when not confident. Targets the
    /// adversarial failure SealQA Seal-0 exposed: 32% confident-WRONG
    /// (hallucination) on conflicting-source / false-premise questions, where
    /// the "use ONLY sources" rule alone is insufficient. Complements (does not
    /// replace) `answer_calibrated`. Default false; A/B requires Seal-0
    /// hallucination DOWN with SimpleQA accuracy NOT regressed before flip.
    #[serde(default)]
    pub answer_guarded: bool,
    /// Use SearXNG structured sources (gated, W0). SearXNG's `infoboxes[]` /
    /// `answers[]` arrays carry Wikidata/Wikipedia knowledge-panel facts
    /// (entity attributes like religion/capital/director) that the `results[]`
    /// transform path discards. With this on, those facts are parsed and pinned
    /// as a high-trust source at the FRONT of the answer pool (still
    /// UNTRUSTED-wrapped — widens evidence, never bypasses the safety wrapper).
    /// Targets the obscure-entity recall gap (PopQA). Default false; A/B on
    /// diag500 gold-in-sources with the wrong-non-abstain invariant before flip.
    #[serde(default)]
    pub use_structured_sources: bool,
    /// Deterministic Wikidata entity-relation lookup (gated, W3). For
    /// `<relation> of <entity>` questions (PopQA's obscure long tail that web
    /// search can't surface), classify -> wbsearchentities -> property fetch and
    /// pin the fact as a structured source (UNTRUSTED-wrapped, runs in parallel
    /// with SearXNG, 3s-bounded, any error falls through). Free open data, no
    /// AI, no SPARQL hot-path. Default false; A/B on diag500 PopQA accuracy +
    /// the wrong-non-abstain invariant before flip.
    #[serde(default)]
    pub wikidata_lookup: bool,
    /// Snippet fallback for the LLM answer path (gated): when a top-N result's
    /// scrape failed (empty `markdown`), the result is normally dropped from the
    /// answer pool — if it was the answer-bearing page, crw abstains though
    /// retrieval succeeded (diagnosed Pattern A). With this on, such results
    /// fall back to their SearXNG `description` snippet as a thin source instead
    /// of vanishing. The snippet is verbatim upstream text, so it cannot inject
    /// a fact not already present — near-zero INCORRECT exposure. Default false.
    #[serde(default)]
    pub snippet_fallback: bool,
    /// Relevance gate for the LLM answer / summarize re-rank (gated). After the
    /// lexical-core junk/coverage/geo filters, keep only the rows that cover the
    /// MOST important (non-stopword) query terms present in the pool, so a
    /// partial-match homonym ("best pizza in REDMOND" for "best pizza in
    /// belgrade", coverage 1/2) is evicted the instant a full-match row
    /// ("pizza … belgrade", 2/2) is present. Ranks on the query's OWN tokens —
    /// no geo/country/IP signal — so it holds for self-hosted deployments in any
    /// region. Monotone-safe (degrade fallback applies first; never empties a
    /// non-empty pool). Requires `rerank_enabled`. Default false; A/B against
    /// the frozen rerank benchmark before flip.
    #[serde(default)]
    pub rerank_relevance: bool,
    /// List-format answers for the LLM answer path (gated). When the query has
    /// list intent ("best/top X in Y", "recommend …", "list of …"), the answer
    /// prompt's prose directive is swapped for a ranked-list directive so the
    /// model emits up to 10 named options (`N. <name> — <why>`) instead of a
    /// 3–6 sentence paragraph. A deterministic classifier (`is_list_intent`)
    /// decides per query; factual/non-list queries are untouched. The "use ONLY
    /// sources" grounding, the abstention rule, and the `===CITATIONS===` block
    /// are preserved (no fabrication, citation moat intact). Default false; A/B
    /// against the answer-accuracy benchmark before flip.
    #[serde(default)]
    pub answer_list_format: bool,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            searxng_url: None,
            openalex_api_key: None,
            openalex_mailto: None,
            s2_api_key: None,
            timeout_ms: default_search_timeout_ms(),
            default_limit: default_search_limit(),
            max_limit: default_search_max_limit(),
            research_engines: default_research_engines(),
            github_engines: default_github_engines(),
            rerank_enabled: true,
            query_expand: false,
            query_expand_variants: default_query_expand_variants(),
            pipeline_overlap: false,
            multi_round: false,
            passage_select: false,
            page2_fallback: false,
            answer_calibrated: false,
            answer_guarded: false,
            use_structured_sources: false,
            wikidata_lookup: false,
            snippet_fallback: false,
            rerank_relevance: false,
            answer_list_format: false,
        }
    }
}

fn default_query_expand_variants() -> usize {
    1
}
fn default_true_search() -> bool {
    true
}
fn default_search_timeout_ms() -> u64 {
    15_000
}
fn default_search_limit() -> u32 {
    5
}
fn default_search_max_limit() -> u32 {
    20
}
fn default_research_engines() -> Vec<String> {
    vec![
        "arxiv".into(),
        "crossref".into(),
        "google scholar".into(),
        "semantic scholar".into(),
    ]
}
fn default_github_engines() -> Vec<String> {
    vec!["github".into()]
}

/// Per-request defaults that apply to every scrape, crawl, or map call when
/// the caller does not specify an override. Currently only governs the
/// end-to-end deadline budget (see `crw-core/src/deadline.rs`).
#[derive(Debug, Clone, Deserialize)]
pub struct RequestConfig {
    /// Default end-to-end deadline budget in milliseconds when a request does
    /// not specify `deadlineMs`. The SLO p95 latency metric is computed only
    /// over requests with `deadline_ms <= 8000`; longer values land in a
    /// separate slow-path histogram.
    #[serde(default = "default_deadline_ms")]
    pub deadline_ms_default: u64,
    /// When `true` (default), an implicit deadline (no per-request `deadlineMs`)
    /// is auto-extended to `max(deadline_ms_default, ladder_min)` where
    /// `ladder_min = sum(http+lightpanda+chrome timeouts) + N_cdp_tiers * 28s`.
    /// This prevents `chrome_timeout_ms = 30000` from appearing inert when
    /// `deadline_ms_default` is small (issue #35).
    ///
    /// Set to `false` to enforce a strict SLO regardless of tier sizing —
    /// requests that would have completed under the extended budget will
    /// instead time out at `deadline_ms_default`.
    #[serde(default = "default_true_request")]
    pub auto_extend_deadline_for_ladder: bool,
}

impl Default for RequestConfig {
    fn default() -> Self {
        Self {
            deadline_ms_default: default_deadline_ms(),
            auto_extend_deadline_for_ladder: true,
        }
    }
}

fn default_true_request() -> bool {
    true
}

fn default_deadline_ms() -> u64 {
    8000
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_request_timeout")]
    pub request_timeout_secs: u64,
    /// Maximum requests per second (global). 0 = unlimited.
    #[serde(default = "default_rate_limit_rps")]
    pub rate_limit_rps: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            request_timeout_secs: default_request_timeout(),
            rate_limit_rps: default_rate_limit_rps(),
        }
    }
}

fn default_rate_limit_rps() -> u64 {
    10
}

fn default_host() -> String {
    "0.0.0.0".into()
}
fn default_port() -> u16 {
    3000
}
fn default_request_timeout() -> u64 {
    60
}

/// Selects which JS renderer(s) the [`FallbackRenderer`] will build.
///
/// - `Auto` (default): try every configured CDP endpoint (Lightpanda, Playwright, Chrome)
///   in order. If none is configured, JS rendering is disabled but HTTP still works.
/// - `None`: HTTP-only. Never attempt JS rendering.
/// - `Lightpanda` / `Chrome` / `Playwright`: require the matching `[renderer.<name>]`
///   endpoint; fail startup if missing. Only the named backend is used.
///
/// [`FallbackRenderer`]: https://docs.rs/crw-renderer/latest/crw_renderer/struct.FallbackRenderer.html
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RendererMode {
    #[default]
    Auto,
    None,
    Lightpanda,
    Chrome,
    Playwright,
    /// Opt-in Camoufox stealth tier (REST, not CDP). Pinning `mode = "camoufox"`
    /// uses only this tier. See [`CamoufoxEndpoint`]. Requires the `camoufox`
    /// build feature; a build without it rejects this mode at renderer
    /// construction time.
    Camoufox,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RendererConfig {
    #[serde(default)]
    pub mode: RendererMode,
    /// Generic per-page navigation timeout. Used as the fallback when no
    /// per-tier override is configured. Kept for backward compatibility — the
    /// per-tier knobs below are preferred for new deployments.
    #[serde(default = "default_page_timeout")]
    pub page_timeout_ms: u64,
    /// Override for the HTTP-only fetcher request timeout. Falls back to
    /// `page_timeout_ms` when unset. HTTP responses arrive quickly when they
    /// arrive at all, so 15s is generous and keeps slow upstreams from
    /// hogging the request budget that should be spent on JS retries.
    #[serde(default)]
    pub http_timeout_ms: Option<u64>,
    /// Override for the LightPanda CDP renderer. LightPanda completes most
    /// renders in <10s; if it stalls past 20s it almost always means an
    /// adversarial page that Chrome will render anyway, so failing fast and
    /// escalating beats waiting it out.
    #[serde(default)]
    pub lightpanda_timeout_ms: Option<u64>,
    /// Override for the full-Chromium tier. Chrome is the slow path
    /// (gov/legal SPAs need 30–40s for `networkidle`); the larger budget here
    /// recovers ~6 URLs per fc-wins iteration without affecting the fast path.
    #[serde(default)]
    pub chrome_timeout_ms: Option<u64>,
    #[serde(default = "default_pool_size")]
    pub pool_size: usize,
    /// latency-qn: override the chrome post-navigate challenge-clear retry count
    /// (default 3 → 3×3s=9s). Measured at 28% of render time, mostly on shells
    /// that never clear (fail anyway); Firecrawl/Spider run no such loop. Lower
    /// to trim the anti-bot tail (e.g. 1); 0 disables it. `None` keeps the 3
    /// default. A/B-gated: must hold scrape-success/recall on the bench.
    #[serde(default)]
    pub chrome_challenge_max_retries: Option<u32>,
    /// latency-qn: override the chrome SPA-readiness poll budget (default 8000ms,
    /// `SPA_SELECTOR_MAX_MS`). Measured at 67% of render time. The poll still
    /// exits early on content-ready/network-idle; this caps the wait when the
    /// selector never mounts. A/B-gated on the bench. `None` keeps 8000.
    #[serde(default)]
    pub chrome_spa_selector_max_ms: Option<u64>,
    /// latency-qn: event-driven earliest-ready render exit. When true, the
    /// post-navigate poll exits as soon as the page is genuinely settled (body
    /// innerText ≥ content-floor AND networkAlmostIdle≤2, OR substantial text)
    /// instead of requiring a specific content selector + networkIdle(0) up to
    /// the 8s ceiling. Keeps a mandatory content floor (never snapshots an empty
    /// shell). Default off; A/B-gated on the bench (quality_gate must hold).
    #[serde(default)]
    pub chrome_fast_ready: bool,
    /// latency-qn: conditional hedge. In auto mode, race lightpanda + chrome
    /// CONCURRENTLY (chrome's clock starts immediately instead of after lightpanda
    /// fails) and take the best by tier priority. Cuts the serial prefix (~3.4s
    /// mean / 5.7s p90) on chrome-bound pages. Bounded by a headroom semaphore
    /// (falls back to serial when the pool is busy) so it can't deadlock the
    /// context pool. best-result-wins ⇒ success/recall ≡ serial. Default off.
    #[serde(default)]
    pub chrome_hedge: bool,
    /// Phase 2 (latency-qn): gated auto-egress escalation. When true, the
    /// chrome_proxy (residential/stealth) tier is REMOVED from the normal
    /// HTTP→LP→Chrome ladder and instead fired ONCE, only when the ladder's
    /// result is a hard block (403/429/503/401/520-530 or a CF/bot-wall/vendor
    /// interstitial) AND the remaining deadline can absorb a full chrome_proxy
    /// attempt AND its breaker is closed. The retry is best-result-wins vs the
    /// ladder's result (never replaces usable content with empty). Bench proved
    /// a naive always-on chrome_proxy ladder is net-negative (success −2pp, p90
    /// +69%); this gate is what makes residential recovery net-positive. Off by
    /// default; requires a configured `[renderer.chrome_proxy]` tier to do anything.
    #[serde(default)]
    pub auto_egress_escalation: bool,
    /// Phase 0 (latency-qn): when true, the renderer emits a structured
    /// `target: "latency_breakdown"` tracing event per fetch with total wall
    /// time and the tier that produced the accepted result. Off by default;
    /// turned on only for bench/diagnostic runs so we can see where the p90
    /// budget actually goes (HTTP fast-path vs JS render) before optimizing.
    #[serde(default)]
    pub latency_breakdown: bool,
    /// If set, applies to every request that doesn't specify `renderJs` explicitly.
    /// `Some(true)` = force JS rendering; `Some(false)` = skip JS; `None` = auto-detect.
    ///
    /// Accepts the `force_js` alias for backward compatibility.
    #[serde(default, alias = "force_js")]
    pub render_js_default: Option<bool>,
    #[serde(default)]
    pub lightpanda: Option<CdpEndpoint>,
    #[serde(default)]
    pub playwright: Option<CdpEndpoint>,
    #[serde(default)]
    pub chrome: Option<CdpEndpoint>,
    /// Residential-proxy Chrome tier (opt-in 4th renderer). Same Chromium
    /// browser as `chrome`, but egress routed through a forwarder that adds
    /// upstream proxy auth (e.g. DataImpulse). Tried after Chrome fails —
    /// covers IP-blocked targets where the browser fingerprint is fine but
    /// the VPS egress IP is flagged.
    #[serde(default)]
    pub chrome_proxy: Option<CdpEndpoint>,
    /// Per-tier nav timeout override for `chrome_proxy`. When unset, defaults
    /// to `chrome_timeout() + 15_000` — the proxy hop adds latency, so the
    /// fallback tier needs more headroom than direct Chrome.
    #[serde(default)]
    pub chrome_proxy_timeout_ms: Option<u64>,
    /// Opt-in Camoufox stealth REST endpoint. See [`CamoufoxEndpoint`].
    /// `None` = not configured (default). Orthogonal to the CDP tiers: a
    /// configured endpoint with the default `include_in_auto = false` does NOT
    /// change the existing auto ladder — it is reachable only via an explicit
    /// per-request `renderer = "camoufox"` pin or `mode = "camoufox"`.
    #[serde(default)]
    pub camoufox: Option<CamoufoxEndpoint>,
    /// Per-request Camoufox REST budget override (ms). Falls back to
    /// [`CAMOUFOX_DEFAULT_TIMEOUT_MS`] when unset.
    #[serde(default)]
    pub camoufox_timeout_ms: Option<u64>,
    /// Enable Chrome resource interception (`Fetch.enable` blocking of media,
    /// fonts, trackers). Default `false`; flipped after the CDP-fake suite
    /// validates pump + cleanup behaviour. See plan Phase 2.
    #[serde(default)]
    pub chrome_intercept_resources: bool,
    /// Additionally block `stylesheet` requests when interception is enabled.
    /// Default `false` — kept off in v1 because some extractors depend on
    /// CSS-driven visibility / lazy-content triggers.
    #[serde(default)]
    pub chrome_intercept_stylesheets: bool,
    /// Per-host opt-out for chrome interception. Hosts in this list run with
    /// interception disabled even when `chrome_intercept_resources = true`.
    #[serde(default)]
    pub chrome_host_intercept_disable: Vec<String>,
    /// Hard chrome-tier navigation budget in ms. Wraps `wait_for_page_ready`
    /// in an inner race; on budget hit the renderer snapshots whatever DOM is
    /// present and returns `truncated = true`. Calibrated as
    /// `p90(successful chrome renders)` clamped to `[8_000, 12_000]`.
    #[serde(default = "default_chrome_nav_budget_ms")]
    pub chrome_nav_budget_ms: u64,
    /// Enable the bounded browser-context pool. Default `false`; v1 ships
    /// `RECYCLE_AFTER_NAV = 1` (recreate every release) before optimising to
    /// reuse-with-clearing. See plan Phase 4. **Gated off when
    /// `chrome_backend = "browserless"`** — browserless v2's
    /// `Target.createBrowserContext` semantics with long-lived sessions are
    /// unproven; lib.rs forces this to `false` with a WARN log in that case.
    #[serde(default)]
    pub chrome_context_pool_enabled: bool,
    /// Per-knob pool configuration. Read only when
    /// `chrome_context_pool_enabled = true` AND backend is `Vanilla`.
    #[serde(default)]
    pub chrome_pool: ChromePoolConfig,
    /// Which Chrome backend the WS URL points at. **Explicit** — never sniff
    /// from URL substrings (k8s svc names, port-forwards, custom routes break
    /// substring detection per plan §C2). Default `Vanilla`.
    #[serde(default)]
    pub chrome_backend: ChromeBackend,
    /// Enable the success-ratio renderer predictor in `HostPreferences`.
    /// Default `false`; flipped after the predictor replay harness gates
    /// on the 1k bench (false-skip < 2 %, false-escalate < 5 %, churn < 3 / 1k).
    #[serde(default)]
    pub use_predictor: bool,
    /// Engine escalation policy (firecrawl-shaped: race + on-error). When
    /// disabled (default), the renderer keeps its current ladder unchanged.
    #[serde(default)]
    pub escalation: EscalationConfig,
    /// Anti-bot detection policy (crawl4ai 3-tier classifier).
    #[serde(default)]
    pub antibot: AntibotConfig,
    /// DataImpulse residential-proxy base username (without `__cr.<cc>`
    /// country suffix). When set alongside [`proxy_base_pass`], the engine
    /// drives Chrome's proxy auth via CDP `Fetch.authRequired` and composes
    /// the country-suffixed username per request. Read only by the
    /// `chrome_proxy` tier. None = no upstream proxy auth (chrome_proxy
    /// tier still functional only if a no-auth or pre-authed proxy is in
    /// front of Chrome).
    #[serde(default)]
    pub proxy_base_user: Option<String>,
    /// DataImpulse base password — see [`proxy_base_user`].
    #[serde(default)]
    pub proxy_base_pass: Option<String>,
    /// Fallback country code used when a request omits `country`. Lowercased
    /// 2-letter ISO 3166-1 alpha-2 (e.g. "us"). None = global pool (no suffix).
    #[serde(default)]
    pub proxy_default_country: Option<String>,
}

/// Engine escalation policy — adds `ChromeStealth` and `ChromeStealthProxy`
/// tiers behind a feature flag. See `plans/recall-next-tier.md` Phase 2.
#[derive(Debug, Clone, Deserialize)]
pub struct EscalationConfig {
    /// Master switch. Default `false` — current ladder runs unchanged.
    #[serde(default)]
    pub enabled: bool,
    /// Per-tier waterfall trigger in ms. If the current engine hasn't returned
    /// after this long, the next tier is started in parallel (firecrawl
    /// `WaterfallNextEngineSignal`).
    #[serde(default = "default_waterfall_timeout_ms")]
    pub waterfall_timeout_ms: u64,
    /// Hard global cap across the whole ladder.
    #[serde(default = "default_escalation_global_timeout_ms")]
    pub global_timeout_ms: u64,
    /// Send `?proxy=residential&proxyCountry=…` to browserless on the
    /// `ChromeStealthProxy` tier. Off by default — bears cost.
    #[serde(default)]
    pub residential_proxy: bool,
    /// Country code passed to browserless when `residential_proxy = true`.
    #[serde(default = "default_proxy_country")]
    pub proxy_country: String,
}

impl Default for EscalationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            waterfall_timeout_ms: default_waterfall_timeout_ms(),
            global_timeout_ms: default_escalation_global_timeout_ms(),
            residential_proxy: false,
            proxy_country: default_proxy_country(),
        }
    }
}

fn default_waterfall_timeout_ms() -> u64 {
    8_000
}
fn default_escalation_global_timeout_ms() -> u64 {
    60_000
}
fn default_proxy_country() -> String {
    "us".to_string()
}

/// Anti-bot classifier policy. Default: detect+log only; escalation requires
/// `escalate_on_signal = true` AND `escalation.enabled = true`.
#[derive(Debug, Clone, Deserialize)]
pub struct AntibotConfig {
    /// Run the classifier inside the renderer failover loop on every fetch
    /// result. Cheap; default on. NOTE: this gates only the in-loop classifier
    /// (see `crw-renderer`); the API-surface block verdict is classified
    /// separately and unconditionally at the scrape choke
    /// (`crw_crawl::single::classify_block`) and is not suppressed by this flag.
    /// To disable in-loop escalation, use `escalate_in_failover`.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// When the classifier returns a non-`None` signal, advance to the next
    /// engine tier (requires `escalation.enabled`).
    #[serde(default)]
    pub escalate_on_signal: bool,
    /// When the classifier flags a block during the renderer failover loop,
    /// treat the result as a soft failure so the loop advances to the next
    /// tier — ending at `chrome_proxy` (residential). Default `true`. Set
    /// `false` to keep the classifier running (error_code + telemetry) while
    /// disabling in-loop escalation — the one-line kill switch.
    #[serde(default = "default_true")]
    pub escalate_in_failover: bool,
}

impl Default for AntibotConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            escalate_on_signal: false,
            escalate_in_failover: true,
        }
    }
}

fn default_chrome_nav_budget_ms() -> u64 {
    12_000
}

/// Per-knob configuration for the bounded browser-context pool. Loaded under
/// `[renderer.chrome_pool]`. Inactive unless
/// `chrome_context_pool_enabled = true` AND `chrome_backend = "vanilla"`.
#[derive(Debug, Clone, Deserialize)]
pub struct ChromePoolConfig {
    /// Pool size. `None` → `max(2, num_cpus / 2)`. Caps simultaneous
    /// in-flight chrome requests per pool.
    #[serde(default)]
    pub size: Option<usize>,
    /// Recycle policy: v1 always recreates the context after each release.
    /// Reserved for a future "reuse N navigations then recreate" mode.
    #[serde(default = "default_recycle_after_navs")]
    pub recycle_after_navs: u32,
    /// Idle slots older than this are health-checked on next acquire.
    #[serde(default = "default_idle_timeout_secs")]
    pub idle_timeout_secs: u64,
    /// `Browser.getVersion` probe deadline (idle-slot liveness).
    #[serde(default = "default_health_check_secs")]
    pub health_check_secs: u64,
    /// SIGTERM drain window before phase 3 force-close.
    #[serde(default = "default_shutdown_drain_secs")]
    pub shutdown_drain_secs: u64,
}

impl Default for ChromePoolConfig {
    fn default() -> Self {
        Self {
            size: None,
            recycle_after_navs: default_recycle_after_navs(),
            idle_timeout_secs: default_idle_timeout_secs(),
            health_check_secs: default_health_check_secs(),
            shutdown_drain_secs: default_shutdown_drain_secs(),
        }
    }
}

fn default_recycle_after_navs() -> u32 {
    1
}
fn default_idle_timeout_secs() -> u64 {
    300
}
fn default_health_check_secs() -> u64 {
    60
}
fn default_shutdown_drain_secs() -> u64 {
    30
}

/// Chrome backend kind. Set explicitly under `[renderer]` as
/// `chrome_backend = "vanilla"` or `chrome_backend = "browserless"`. **Never
/// inferred from URL substrings** — k8s service names, port-forwards, and
/// custom routes break substring detection. See plan §C2.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ChromeBackend {
    /// chromedp/headless-shell or vanilla Chrome with `/json/version`. Pool
    /// is enabled here when `chrome_context_pool_enabled = true`.
    #[default]
    Vanilla,
    /// Browserless v2 / commercial CDP endpoint. Pool is **gated off** in v1
    /// — see plan §"Out of scope (v1)".
    Browserless,
}

impl Default for RendererConfig {
    fn default() -> Self {
        Self {
            mode: RendererMode::default(),
            page_timeout_ms: default_page_timeout(),
            http_timeout_ms: None,
            lightpanda_timeout_ms: None,
            chrome_timeout_ms: None,
            pool_size: default_pool_size(),
            chrome_challenge_max_retries: None,
            chrome_spa_selector_max_ms: None,
            chrome_fast_ready: false,
            chrome_hedge: false,
            auto_egress_escalation: false,
            latency_breakdown: false,
            render_js_default: None,
            lightpanda: None,
            playwright: None,
            chrome: None,
            chrome_proxy: None,
            chrome_proxy_timeout_ms: None,
            camoufox: None,
            camoufox_timeout_ms: None,
            chrome_intercept_resources: false,
            chrome_intercept_stylesheets: false,
            chrome_host_intercept_disable: Vec::new(),
            chrome_nav_budget_ms: default_chrome_nav_budget_ms(),
            chrome_context_pool_enabled: false,
            chrome_pool: ChromePoolConfig::default(),
            chrome_backend: ChromeBackend::default(),
            use_predictor: false,
            escalation: EscalationConfig::default(),
            antibot: AntibotConfig::default(),
            proxy_base_user: None,
            proxy_base_pass: None,
            proxy_default_country: None,
        }
    }
}
fn default_page_timeout() -> u64 {
    30000
}

impl RendererConfig {
    /// Resolved per-tier nav timeout in milliseconds. Resolution rules:
    ///   1. If the explicit per-tier field is set, use it verbatim.
    ///   2. Otherwise fall back to `page_timeout_ms` (which itself defaults
    ///      to 30s for backward compatibility with pre-multi-tier configs).
    ///
    /// New deployments are encouraged to set the per-tier knobs to 15/20/45s
    /// (see config.docker.toml) — these match the bench-tuned values that
    /// recover slow gov sites in the chrome tier without giving the http
    /// tier permission to hog the request budget.
    pub fn http_timeout(&self) -> u64 {
        self.http_timeout_ms.unwrap_or(self.page_timeout_ms)
    }
    pub fn lightpanda_timeout(&self) -> u64 {
        self.lightpanda_timeout_ms.unwrap_or(self.page_timeout_ms)
    }
    pub fn chrome_timeout(&self) -> u64 {
        self.chrome_timeout_ms.unwrap_or(self.page_timeout_ms)
    }
    pub fn chrome_proxy_timeout(&self) -> u64 {
        self.chrome_proxy_timeout_ms
            .unwrap_or_else(|| self.chrome_timeout().saturating_add(15_000))
    }
    pub fn camoufox_timeout(&self) -> u64 {
        self.camoufox_timeout_ms
            .unwrap_or(CAMOUFOX_DEFAULT_TIMEOUT_MS)
    }

    /// True when the Camoufox REST tier participates in the *auto* ladder for
    /// the current mode — i.e. it would be tried for a non-pinned request.
    /// Distinct from merely "configured": a configured endpoint with
    /// `include_in_auto = false` returns `false` here unless `mode == Camoufox`.
    ///
    /// | mode                              | result                          |
    /// |-----------------------------------|---------------------------------|
    /// | `None`                            | `false` (no renderers at all)   |
    /// | `Camoufox`                        | `true` when configured (pinned) |
    /// | `Auto` + `include_in_auto = true` | `true`                          |
    /// | `Auto` + `include_in_auto = false`| `false` (opt-in default)        |
    /// | `Lightpanda`/`Chrome`/`Playwright`| `false`                         |
    ///
    /// This method is intentionally NOT `#[cfg(feature = "camoufox")]`: callers
    /// in the deadline math (`min_deadline_for_full_ladder_ms`,
    /// `effective_deadline_ms`) reference it unconditionally. The leading
    /// `cfg!(feature = "camoufox")` runtime check (compiled to a constant and
    /// dead-code-eliminated by LLVM in the lean build) makes it always return
    /// `false` without the feature, so HTTP-only builds stay byte-identical.
    /// Do not remove that early return thinking it is redundant — it is what
    /// keeps `RendererMode::Camoufox` (an unconditional enum variant) inert in
    /// lean builds.
    pub fn camoufox_in_ladder(&self) -> bool {
        if !cfg!(feature = "camoufox") || matches!(self.mode, RendererMode::None) {
            return false;
        }
        // Mirror the construction filter in `FallbackRenderer::new`, which only
        // builds the tier when `base_url` is non-empty. Without this guard a
        // degenerate config (blank `base_url` + `include_in_auto = true`) would
        // claim ladder membership for a tier that is never constructed, leaking
        // a phantom +camoufox_timeout into the deadline budget.
        let configured = |c: &CamoufoxEndpoint| !c.base_url.trim().is_empty();
        match self.mode {
            RendererMode::Camoufox => self.camoufox.as_ref().is_some_and(configured),
            RendererMode::Auto => self
                .camoufox
                .as_ref()
                .is_some_and(|c| c.include_in_auto && configured(c)),
            _ => false,
        }
    }

    /// Compose the DataImpulse-style proxy credentials for a single request.
    ///
    /// Resolution order for the country suffix:
    /// 1. `country` argument (per-request override)
    /// 2. `self.proxy_default_country` (server default)
    /// 3. No suffix → DataImpulse global pool
    ///
    /// Returns `None` when no base credentials are configured — caller treats
    /// this as "no auth required". An invalid country code (wrong length,
    /// non-alphabetic) silently falls through to the default; that keeps a
    /// malformed `?country=` query from creating an unauthenticated request
    /// while still letting through a well-known default.
    pub fn effective_proxy_credentials(&self, country: Option<&str>) -> Option<(String, String)> {
        let user = self.proxy_base_user.as_ref()?;
        let pass = self.proxy_base_pass.as_ref()?;
        let cc = country
            .or(self.proxy_default_country.as_deref())
            .map(|s| s.trim().to_lowercase())
            .filter(|s| s.len() == 2 && s.chars().all(|c| c.is_ascii_alphabetic()));
        Some(match cc {
            Some(cc) => (format!("{user}__cr.{cc}"), pass.clone()),
            None => (user.clone(), pass.clone()),
        })
    }

    /// Number of active CDP tiers (lightpanda + playwright + chrome) under
    /// the current `mode`. Mirrors the predicate used at runtime in
    /// `crw-renderer/src/lib.rs` when constructing the renderer ladder:
    /// `want(mode) && config.<tier>.is_some()`.
    ///
    /// Returns `0` when the binary is built without the `cdp` feature — in
    /// that case no JS renderer can be constructed regardless of the config,
    /// so the deadline auto-extension policy must collapse to HTTP-only.
    pub fn cdp_tier_count(&self) -> usize {
        if !cfg!(feature = "cdp") {
            return 0;
        }
        let want =
            |m: RendererMode| -> bool { matches!(self.mode, RendererMode::Auto) || self.mode == m };
        let mut n = 0;
        if want(RendererMode::Lightpanda) && self.lightpanda.is_some() {
            n += 1;
        }
        if want(RendererMode::Playwright) && self.playwright.is_some() {
            n += 1;
        }
        if want(RendererMode::Chrome) && self.chrome.is_some() {
            n += 1;
        }
        n
    }

    /// Minimum request deadline budget (ms) required so that every configured
    /// tier can use its full allowance when fallback exhausts the chain.
    /// Sums the per-tier timeouts and adds [`CDP_TIER_OVERHEAD_MS`] for each
    /// active CDP tier, matching the runtime ladder built in
    /// `crw-renderer/src/lib.rs`.
    pub fn min_deadline_for_full_ladder_ms(&self) -> u64 {
        let want =
            |m: RendererMode| -> bool { matches!(self.mode, RendererMode::Auto) || self.mode == m };

        let mut sum: u64 = 0;
        // HTTP prefetch runs ahead of any JS tier (content-type sniffing,
        // direct PDF/binary handling) regardless of pinned mode. Skipped only
        // when mode is `None` (no fetching at all).
        if !matches!(self.mode, RendererMode::None) {
            sum = sum.saturating_add(self.http_timeout());
        }

        // Camoufox REST contribution. Added BEFORE the cdp early-return below so
        // an HTTP-only + camoufox-enabled build still extends the deadline.
        // Camoufox is a single REST tier: it is never counted in
        // `cdp_tier_count` and never charged `CDP_TIER_OVERHEAD_MS`.
        // `camoufox_in_ladder()` is always `false` in the lean build, so this
        // line is inert there.
        if self.camoufox_in_ladder() {
            sum = sum.saturating_add(self.camoufox_timeout());
        }

        // CDP tiers only contribute when the binary was built with the `cdp`
        // feature; otherwise no JS renderer is constructable at runtime and
        // including their budgets would over-extend the deadline.
        if !cfg!(feature = "cdp") {
            return sum;
        }

        let mut cdp_tier_count: u64 = 0;
        if want(RendererMode::Lightpanda) && self.lightpanda.is_some() {
            sum = sum.saturating_add(self.lightpanda_timeout());
            cdp_tier_count += 1;
        }
        if want(RendererMode::Playwright) && self.playwright.is_some() {
            sum = sum.saturating_add(self.chrome_timeout());
            cdp_tier_count += 1;
        }
        if want(RendererMode::Chrome) && self.chrome.is_some() {
            sum = sum.saturating_add(self.chrome_timeout());
            cdp_tier_count += 1;
        }
        sum.saturating_add(cdp_tier_count.saturating_mul(CDP_TIER_OVERHEAD_MS))
    }
}
fn default_pool_size() -> usize {
    4
}

#[derive(Debug, Clone, Deserialize)]
pub struct CdpEndpoint {
    pub ws_url: String,
}

/// Opt-in Camoufox stealth renderer endpoint (REST, not CDP). When present it
/// is selectable via `mode = "camoufox"` or a per-request `renderer =
/// "camoufox"` pin, and additionally joins the Auto ladder ONLY when
/// `include_in_auto = true`. A configured endpoint with the default
/// `include_in_auto = false` does NOT change the existing auto ladder.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CamoufoxEndpoint {
    /// Base URL of the camofox-browser REST server, e.g. `http://localhost:9377`.
    pub base_url: String,
    /// Optional bearer token sent as `Authorization: Bearer <key>`. Empty string
    /// (the default) means no auth header is added.
    #[serde(default)]
    pub api_key: String,
    /// Whether this tier joins the Auto fallback ladder. Default `false`:
    /// configured-but-not-in-auto, reachable only via an explicit pin or
    /// `mode = "camoufox"`.
    #[serde(default)]
    pub include_in_auto: bool,
}

/// Stealth mode configuration for evading bot detection.
#[derive(Debug, Clone, Deserialize)]
pub struct StealthConfig {
    /// Enable stealth mode globally.
    #[serde(default)]
    pub enabled: bool,
    /// Custom user-agent pool. Empty = use built-in pool.
    #[serde(default)]
    pub user_agents: Vec<String>,
    /// Jitter factor for rate limiting (0.0–1.0, default 0.2 = ±20%).
    #[serde(default = "default_jitter")]
    pub jitter_factor: f64,
    /// Inject realistic browser headers (Accept, Sec-Fetch-*, etc.).
    #[serde(default = "default_true")]
    pub inject_headers: bool,
}

impl Default for StealthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            user_agents: vec![],
            jitter_factor: default_jitter(),
            inject_headers: true,
        }
    }
}

fn default_jitter() -> f64 {
    0.2
}

/// Built-in realistic user-agent pool used when stealth is enabled.
pub const BUILTIN_UA_POOL: &[&str] = &[
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:140.0) Gecko/20100101 Firefox/140.0",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_5) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 Safari/605.1.15",
];

#[derive(Debug, Clone, Deserialize)]
pub struct CrawlerConfig {
    #[serde(default = "default_concurrency")]
    pub max_concurrency: usize,
    #[serde(default = "default_rps")]
    pub requests_per_second: f64,
    #[serde(default = "default_true")]
    pub respect_robots_txt: bool,
    #[serde(default = "default_ua")]
    pub user_agent: String,
    #[serde(default = "default_depth")]
    pub default_max_depth: u32,
    #[serde(default = "default_max_pages")]
    pub default_max_pages: u32,
    /// Proxy URL for crawler requests. Supports HTTP, HTTPS, and SOCKS5
    /// (e.g. "http://proxy:8080" or "socks5://user:pass@proxy:1080"). An empty
    /// or whitespace-only value (e.g. a present-but-empty `CRW_CRAWLER__PROXY`)
    /// is normalized to `None` — see [`deserialize_opt_nonempty_string`].
    #[serde(default, deserialize_with = "deserialize_opt_nonempty_string")]
    pub proxy: Option<String>,
    /// Pool of proxy URLs to rotate among (HTTP, HTTPS, SOCKS5). When non-empty
    /// this takes precedence over the single `proxy` field. Empty (default) =
    /// no rotation. Accepts a TOML array, a JSON-array string, or a
    /// comma-separated string (for `CRW_CRAWLER__PROXY_LIST`).
    #[serde(default, deserialize_with = "deserialize_string_vec")]
    pub proxy_list: Vec<String>,
    /// Strategy for selecting from `proxy_list`: `round_robin`, `random`, or
    /// `sticky_per_host` (default). Ignored when the list is empty.
    #[serde(default)]
    pub proxy_rotation: crate::proxy::ProxyRotation,
    /// TTL in seconds for completed crawl jobs before cleanup (default: 3600)
    #[serde(default = "default_job_ttl")]
    pub job_ttl_secs: u64,
    #[serde(default)]
    pub stealth: StealthConfig,
    /// Floor for the per-host limiter interval, in milliseconds. When a host
    /// advertises `Crawl-delay` in robots.txt, the higher of the two wins.
    /// Default `0` — robots.txt is the authoritative source, this is a
    /// per-deployment safety net.
    #[serde(default)]
    pub per_host_min_interval_ms: u64,
    /// Maximum concurrent in-flight requests against a single eTLD+1.
    /// Default `1` — strict ethics posture; operators raise consciously via
    /// config when scraping their own infrastructure.
    #[serde(default = "default_per_host_max_concurrent")]
    pub per_host_max_concurrent: u32,
}

fn default_per_host_max_concurrent() -> u32 {
    1
}

impl Default for CrawlerConfig {
    fn default() -> Self {
        Self {
            max_concurrency: default_concurrency(),
            requests_per_second: default_rps(),
            respect_robots_txt: true,
            user_agent: default_ua(),
            default_max_depth: default_depth(),
            default_max_pages: default_max_pages(),
            proxy: None,
            proxy_list: Vec::new(),
            proxy_rotation: crate::proxy::ProxyRotation::default(),
            job_ttl_secs: default_job_ttl(),
            stealth: StealthConfig::default(),
            per_host_min_interval_ms: 0,
            per_host_max_concurrent: default_per_host_max_concurrent(),
        }
    }
}

fn default_concurrency() -> usize {
    10
}
fn default_rps() -> f64 {
    10.0
}
fn default_true() -> bool {
    true
}
fn default_ua() -> String {
    // Modern Chrome UA. The legacy "CRW/0.1" was rejected by UA-filtering sites
    // (opencorporates, killeenisd, wsj) returning 403/404. Kept in sync with the
    // Sec-Ch-Ua client hint in `crw-renderer/src/http_only.rs`.
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36"
        .into()
}
fn default_depth() -> u32 {
    2
}
fn default_max_pages() -> u32 {
    100
}
fn default_job_ttl() -> u64 {
    3600
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExtractionConfig {
    #[serde(default = "default_format")]
    pub default_format: String,
    #[serde(default = "default_true_ext")]
    pub only_main_content: bool,
    #[serde(default)]
    pub llm: Option<LlmConfig>,
    /// Hostname → CSS selector overrides applied before readability narrowing.
    /// Match is exact host (no wildcard); user-supplied selector still wins.
    #[serde(default)]
    pub domain_selectors: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub llm_fallback: LlmFallbackConfig,
    /// Bytes below which an HTTP-tier extraction is treated as "thin"
    /// and triggers a JS-renderer escalation. Default 100.
    #[serde(default = "default_http_retry_threshold")]
    pub http_retry_threshold_bytes: usize,
    /// Bytes below which a LightPanda-tier extraction is treated as
    /// "thin" and triggers a Chrome escalation. Default 2000 (LP often
    /// returns SPA husks of 90–500B that pass HTML-shape checks).
    #[serde(default = "default_lightpanda_retry_threshold")]
    pub lightpanda_retry_threshold_bytes: usize,
    /// Process-wide cap on concurrent HTML → markdown extractions (html5ever +
    /// htmd). Extraction is CPU-bound and runs on the blocking pool; this bound
    /// keeps a burst of concurrent scrapes from oversubscribing the cores and
    /// starving the async reactor. Defaults to ~2/3 of available cores (≈8 on a
    /// 12-vCPU host), floored at 2.
    #[serde(default = "default_max_concurrent_extracts")]
    pub max_concurrent_extracts: usize,
}

fn default_http_retry_threshold() -> usize {
    100
}

fn default_lightpanda_retry_threshold() -> usize {
    2000
}

fn default_max_concurrent_extracts() -> usize {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    (cpus * 2 / 3).max(2)
}

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self {
            default_format: default_format(),
            only_main_content: true,
            llm: None,
            domain_selectors: std::collections::HashMap::new(),
            llm_fallback: LlmFallbackConfig::default(),
            http_retry_threshold_bytes: default_http_retry_threshold(),
            lightpanda_retry_threshold_bytes: default_lightpanda_retry_threshold(),
            max_concurrent_extracts: default_max_concurrent_extracts(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmFallbackConfig {
    #[serde(default)]
    pub enable: bool,
    #[serde(default = "default_llm_quality_threshold")]
    pub quality_threshold: f32,
    #[serde(default = "default_llm_max_html_bytes")]
    pub max_html_bytes: usize,
    /// When true (and `enable` is true), invoke the LLM on every page rather
    /// than only when DOM-based extraction scores below `quality_threshold`.
    /// Mirrors the "LLM as primary extractor" pattern used by Reader-LM,
    /// Firecrawl, and similar services. Higher cost, higher recall.
    #[serde(default)]
    pub always_run: bool,
}

impl Default for LlmFallbackConfig {
    fn default() -> Self {
        Self {
            enable: false,
            quality_threshold: default_llm_quality_threshold(),
            max_html_bytes: default_llm_max_html_bytes(),
            always_run: false,
        }
    }
}

fn default_llm_quality_threshold() -> f32 {
    0.3
}
fn default_llm_max_html_bytes() -> usize {
    100_000
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmConfig {
    #[serde(default = "default_llm_provider")]
    pub provider: String,
    pub api_key: String,
    #[serde(default = "default_llm_model")]
    pub model: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default = "default_llm_max_tokens")]
    pub max_tokens: u32,
    /// Azure OpenAI API version (e.g. "2024-05-01-preview"). Required when
    /// `provider = "azure"`; ignored otherwise.
    #[serde(default)]
    pub azure_api_version: Option<String>,
    /// Max parallel LLM calls for fan-out (e.g. per-result search summaries).
    /// Bounded to avoid hitting provider rate limits.
    #[serde(default = "default_llm_max_concurrency")]
    pub max_concurrency: usize,
    /// Byte cap on content sent to the LLM in a single call. Content beyond
    /// the cap is truncated on a UTF-8 char boundary.
    #[serde(default = "default_llm_max_html_bytes")]
    pub max_html_bytes: usize,
    /// When set, opencore refuses LLM-touching requests that lack this header
    /// AND do not supply `llm_api_key` in the body. SaaS deploys set this so
    /// direct public callers can't access LLM features.
    #[serde(default)]
    pub require_byok_header: Option<String>,
    /// Sampling temperature for the LLM call. `None` (default) sends no
    /// `temperature` key, preserving each provider's default (DeepSeek = 1) and
    /// current prod behavior. The benchmark/eval harness sets `0.0` (with a
    /// seed) to make answers deterministic so a real +2-3pp lever is
    /// distinguishable from sampling noise. Prod stays `None` until temp=0 is
    /// proven not to raise abstention.
    #[serde(default)]
    pub temperature: Option<f32>,
    /// Optional reasoning-effort hint forwarded to OpenAI-compatible providers
    /// that support it. `None` (default) and the empty string send no key,
    /// preserving each provider's default behavior.
    #[serde(default)]
    pub reasoning_effort: Option<String>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: default_llm_provider(),
            api_key: String::new(),
            model: default_llm_model(),
            base_url: None,
            max_tokens: default_llm_max_tokens(),
            azure_api_version: None,
            max_concurrency: default_llm_max_concurrency(),
            max_html_bytes: default_llm_max_html_bytes(),
            require_byok_header: None,
            temperature: None,
            reasoning_effort: None,
        }
    }
}

fn default_llm_max_concurrency() -> usize {
    4
}

fn default_llm_provider() -> String {
    "anthropic".into()
}
fn default_llm_model() -> String {
    "claude-sonnet-4-20250514".into()
}
fn default_llm_max_tokens() -> u32 {
    4096
}

fn default_format() -> String {
    "markdown".into()
}
fn default_true_ext() -> bool {
    true
}

/// Custom deserializer for Vec<String> that accepts:
/// - TOML array: `api_keys = ["key1", "key2"]`
/// - JSON array: `["key1", "key2"]` (for env vars)
/// - Comma-separated: `key1,key2` (for simple env var usage)
fn deserialize_string_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum StringOrVec {
        Vec(Vec<String>),
        Str(String),
    }

    match StringOrVec::deserialize(deserializer)? {
        StringOrVec::Vec(v) => Ok(v),
        StringOrVec::Str(s) => {
            let s = s.trim();
            // Try JSON array first
            if s.starts_with('[') {
                serde_json::from_str(s).map_err(serde::de::Error::custom)
            } else {
                // Comma-separated fallback
                Ok(s.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect())
            }
        }
    }
}

/// Deserializer for an optional string that normalizes an empty or
/// whitespace-only value to `None`.
///
/// Env-based config (the `config` crate with `try_parsing`) surfaces a
/// present-but-empty variable such as `CRW_CRAWLER__PROXY=""` as `Some("")`
/// rather than `None`. Left as `Some("")`, that empty string flows into
/// `reqwest::Proxy::all("")`, which rejects it with "builder error" and breaks
/// the map/crawl discovery path (issue #154). This mirrors how
/// [`deserialize_string_vec`] already drops empty entries for `proxy_list`.
///
/// Applied via `#[serde(default, deserialize_with = ...)]`, so a *missing* key
/// is handled by `Default` (the helper never runs) and only a present value —
/// from env or a TOML `proxy = ""` — reaches this function.
fn deserialize_opt_nonempty_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    let trimmed = s.trim();
    Ok(if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    })
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AuthConfig {
    #[serde(default, deserialize_with = "deserialize_string_vec")]
    pub api_keys: Vec<String>,
}

/// Path of the per-user config file written by `crw setup`. Returns `None` if
/// the home directory cannot be resolved (e.g. headless container with no
/// `$HOME`). Honors `$CRW_USER_CONFIG_DIR` for tests so we don't have to
/// monkey-patch `$HOME`.
pub fn user_config_path() -> Option<std::path::PathBuf> {
    if let Ok(dir) = std::env::var("CRW_USER_CONFIG_DIR") {
        return Some(std::path::PathBuf::from(dir).join("config.toml"));
    }
    let home = std::env::var_os("HOME")?;
    Some(
        std::path::PathBuf::from(home)
            .join(".config")
            .join("crw")
            .join("config.toml"),
    )
}

impl AppConfig {
    /// Load config from config.default.toml + per-user config + environment
    /// variable overrides.
    ///
    /// Precedence (highest wins):
    ///   1. `CRW_*` env vars (CI/Docker)
    ///   2. `$CRW_CONFIG` file (or `config.local.toml` in cwd)
    ///   3. `~/.config/crw/config.toml` (written by `crw setup`)
    ///   4. `config.default.toml` (bundled defaults)
    ///
    /// Env stays on top so a one-off `CRW_FOO=bar crw …` always wins over
    /// whatever the user has saved, matching how every other shell tool works.
    pub fn load() -> Result<Self, config::ConfigError> {
        let mut builder = config::Config::builder()
            .add_source(config::File::with_name("config.default").required(false));

        // User-level config — written atomically by `crw setup`. Optional, so
        // a never-configured machine simply reads defaults + env.
        if let Some(user_cfg) = user_config_path()
            && user_cfg.exists()
        {
            builder = builder.add_source(config::File::from(user_cfg).required(false));
        }

        // Load optional override config file (e.g. config.docker.toml in containers).
        if let Ok(extra) = std::env::var("CRW_CONFIG") {
            builder = builder.add_source(config::File::with_name(&extra).required(true));
        } else {
            builder = builder.add_source(config::File::with_name("config.local").required(false));
        }

        let cfg = builder
            .add_source(
                config::Environment::with_prefix("CRW")
                    .prefix_separator("_")
                    .separator("__")
                    .try_parsing(true),
            )
            .build()?;
        cfg.try_deserialize()
    }

    /// Compute the effective end-to-end request deadline (ms). Implements the
    /// issue-#35 auto-extension policy:
    ///
    /// 1. If the caller supplied an explicit `requested_deadline_ms`, return it
    ///    verbatim — operators trust the request budget over our heuristic.
    /// 2. Otherwise, when `request.auto_extend_deadline_for_ladder` is on,
    ///    return `max(deadline_ms_default, ladder_min + wait_for_extra)`.
    ///    `ladder_min` covers the configured tier ladder; `wait_for_extra`
    ///    compensates for callers that bumped `wait_for_ms` above the default
    ///    SPA budget (8s) — without it, a long `wait_for` would silently
    ///    re-clamp inside CDP.
    /// 3. When the policy is disabled, return `deadline_ms_default` unchanged.
    ///
    /// `wait_for_ms` is the per-request override (ScrapeRequest::wait_for /
    /// CrawlRequest::wait_for); pass `None` for sub-fetches that don't
    /// surface a wait_for to the caller (search/map enrichment).
    pub fn effective_deadline_ms(
        &self,
        requested_deadline_ms: Option<u64>,
        wait_for_ms: Option<u64>,
    ) -> u64 {
        if let Some(explicit) = requested_deadline_ms {
            return explicit;
        }
        let default_ms = self.request.deadline_ms_default;
        if !self.request.auto_extend_deadline_for_ladder {
            return default_ms;
        }
        // Issue #35 is specifically about CDP tier overhead silently clamping
        // chrome_timeout_ms. HTTP-only deployments don't suffer the same
        // problem (the HTTP renderer respects deadline.remaining without the
        // extra fetch/challenge/stability overhead). Skip the extension when
        // no CDP tiers are configured so HTTP-only users keep the strict
        // operator-configured default.
        //
        // The opt-in Camoufox REST tier also warrants the extension when it is
        // in the ladder (e.g. a camoufox-only, no-CDP deployment) — otherwise
        // its 60s budget would be clamped to the strict default and starved.
        // `camoufox_in_ladder()` is always `false` in the lean build, so
        // HTTP-only deployments keep byte-identical behaviour here.
        if self.renderer.cdp_tier_count() == 0 && !self.renderer.camoufox_in_ladder() {
            return default_ms;
        }
        let ladder_min = self.renderer.min_deadline_for_full_ladder_ms();
        // Mirrors crw_renderer::cdp::SPA_SELECTOR_MAX_MS. The CDP module
        // adds `wait_for_ms.unwrap_or(SPA_SELECTOR_MAX_MS)` to its internal
        // timeout, so when the caller exceeds the default we need to extend
        // the deadline per active CDP tier.
        const SPA_DEFAULT_MS: u64 = 8_000;
        // Clamp `wait_for_ms` to MAX_WAIT_FOR_MS so the inner deadline never
        // exceeds the Tower envelope, which is sized off the same constant in
        // `effective_request_timeout_secs`. A pathological caller passing
        // `wait_for: 600_000` without `deadlineMs` would otherwise be cancelled
        // by Tower before the inner CDP loop noticed the bigger budget.
        let extra = if let Some(w) = wait_for_ms {
            let bounded = w.min(MAX_WAIT_FOR_MS);
            let per_tier = bounded.saturating_sub(SPA_DEFAULT_MS);
            per_tier.saturating_mul(self.renderer.cdp_tier_count() as u64)
        } else {
            0
        };
        default_ms.max(ladder_min.saturating_add(extra))
    }

    /// Tower middleware outer timeout (seconds). Must accommodate the longest
    /// legitimate handler runtime so a healthy request isn't cancelled by the
    /// outer layer before the inner deadline fires.
    ///
    /// Covers the three route envelopes:
    /// - `/scrape`, `/mcp` — auto-extended scrape deadline.
    /// - `/search` — SearXNG fetch + bounded enrichment fan-out
    ///   (`ceil(max_limit / max_concurrency)` batches × scrape_ms).
    /// - `/crawl/jobs/:id`, `/map` — handler-side caps up to 300s.
    ///
    /// When auto-extend is disabled, returns the operator-configured baseline
    /// unchanged.
    pub fn effective_request_timeout_secs(&self) -> u64 {
        let baseline = self.server.request_timeout_secs;
        if !self.request.auto_extend_deadline_for_ladder {
            return baseline;
        }
        const OUTER_BUFFER_SECS: u64 = 5;
        // `/map` handler caps `req.timeout.unwrap_or(120).min(300)`; the outer
        // must cover the upper bound so callers passing `timeout=300` aren't
        // cancelled mid-flight.
        const MAP_REQUEST_TIMEOUT_CEILING_MS: u64 = 300_000;
        // Cover the worst-case implicit scrape: caller bumps `wait_for` to the
        // configured maximum without supplying `deadlineMs`. The same
        // [`MAX_WAIT_FOR_MS`] constant is used inside `effective_deadline_ms`
        // to clamp the inner extension, so the inner deadline can never
        // exceed this outer envelope.
        let scrape_ms = self.effective_deadline_ms(None, Some(MAX_WAIT_FOR_MS));

        // Search enrichment: bounded by max_concurrency. Worst case sequential
        // batching with low concurrency: ceil(max_limit / max_concurrency)
        // batches each bounded by scrape_ms.
        let conc = (self.crawler.max_concurrency.max(1)) as u64;
        let max_results = self.search.max_limit as u64;
        let enrich_batches = max_results.div_ceil(conc);
        let search_enrichment_ms = enrich_batches.saturating_mul(scrape_ms);
        let search_ms = self.search.timeout_ms.saturating_add(search_enrichment_ms);

        let max_handler_ms = scrape_ms.max(search_ms).max(MAP_REQUEST_TIMEOUT_CEILING_MS);
        let needed_secs = max_handler_ms
            .div_ceil(1_000)
            .saturating_add(OUTER_BUFFER_SECS);
        baseline.max(needed_secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Env var tests modify process-wide state; serialize them to avoid cross-test
    /// interference (e.g. `force_js` alias + `render_js_default` direct both set).
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn clear_renderer_env() {
        for k in [
            "CRW_RENDERER__MODE",
            "CRW_RENDERER__FORCE_JS",
            "CRW_RENDERER__RENDER_JS_DEFAULT",
            "CRW_RENDERER__LIGHTPANDA__WS_URL",
            "CRW_RENDERER__CAMOUFOX__BASE_URL",
            "CRW_RENDERER__CAMOUFOX__API_KEY",
            "CRW_RENDERER__CAMOUFOX__INCLUDE_IN_AUTO",
            "CRW_SERVER__PORT",
        ] {
            unsafe { std::env::remove_var(k) };
        }
    }

    #[test]
    fn renderer_mode_parses_variants() {
        #[derive(Deserialize)]
        struct Wrap {
            mode: RendererMode,
        }
        let cases = [
            ("mode = \"auto\"", RendererMode::Auto),
            ("mode = \"none\"", RendererMode::None),
            ("mode = \"lightpanda\"", RendererMode::Lightpanda),
            ("mode = \"chrome\"", RendererMode::Chrome),
            ("mode = \"playwright\"", RendererMode::Playwright),
            ("mode = \"camoufox\"", RendererMode::Camoufox),
        ];
        for (toml_str, expected) in cases {
            let w: Wrap = toml::from_str(toml_str).unwrap();
            assert_eq!(w.mode, expected, "toml: {toml_str}");
        }
    }

    #[test]
    fn renderer_mode_bogus_errors() {
        #[derive(Deserialize)]
        struct Wrap {
            #[allow(dead_code)]
            mode: RendererMode,
        }
        let err: Result<Wrap, _> = toml::from_str("mode = \"bogus\"");
        assert!(err.is_err(), "bogus mode should fail to parse");
    }

    #[test]
    fn renderer_config_default_mode_is_auto() {
        let cfg = RendererConfig::default();
        assert_eq!(cfg.mode, RendererMode::Auto);
        assert_eq!(cfg.render_js_default, None);
    }

    #[cfg(feature = "camoufox")]
    #[test]
    fn camoufox_in_ladder_semantics() {
        let ep = || CamoufoxEndpoint {
            base_url: "http://localhost:9377".into(),
            api_key: String::new(),
            include_in_auto: false,
        };
        // (1) configured + Auto + include_in_auto=false -> NOT in auto ladder.
        let c = RendererConfig {
            mode: RendererMode::Auto,
            camoufox: Some(ep()),
            ..Default::default()
        };
        assert!(
            !c.camoufox_in_ladder(),
            "opt-in default must stay out of auto"
        );
        // (2) Auto + include_in_auto=true -> in ladder.
        let c = RendererConfig {
            mode: RendererMode::Auto,
            camoufox: Some(CamoufoxEndpoint {
                include_in_auto: true,
                ..ep()
            }),
            ..Default::default()
        };
        assert!(c.camoufox_in_ladder());
        // (3) mode=Camoufox pin + include_in_auto=false -> in ladder (pinned).
        let c = RendererConfig {
            mode: RendererMode::Camoufox,
            camoufox: Some(ep()),
            ..Default::default()
        };
        assert!(c.camoufox_in_ladder());
        // (4) mode=None -> never.
        let c = RendererConfig {
            mode: RendererMode::None,
            camoufox: Some(ep()),
            ..Default::default()
        };
        assert!(!c.camoufox_in_ladder());
        // (5) other CDP-pinned modes -> never.
        let c = RendererConfig {
            mode: RendererMode::Chrome,
            camoufox: Some(CamoufoxEndpoint {
                include_in_auto: true,
                ..ep()
            }),
            ..Default::default()
        };
        assert!(!c.camoufox_in_ladder());
        // (6) blank base_url must NOT count as in-ladder even with the flag set
        // (mirrors the construction filter — no phantom deadline extension).
        let c = RendererConfig {
            mode: RendererMode::Auto,
            camoufox: Some(CamoufoxEndpoint {
                base_url: "   ".into(),
                include_in_auto: true,
                ..ep()
            }),
            ..Default::default()
        };
        assert!(!c.camoufox_in_ladder());
        // (7) blank base_url + mode=camoufox pin -> also not in ladder.
        let c = RendererConfig {
            mode: RendererMode::Camoufox,
            camoufox: Some(CamoufoxEndpoint {
                base_url: String::new(),
                ..ep()
            }),
            ..Default::default()
        };
        assert!(!c.camoufox_in_ladder());
    }

    #[cfg(not(feature = "camoufox"))]
    #[test]
    fn camoufox_in_ladder_always_false_without_feature() {
        // Without the feature the tier can never join the ladder, even if a
        // (deserialized) endpoint is present and mode is pinned to camoufox.
        let c = RendererConfig {
            mode: RendererMode::Camoufox,
            camoufox: Some(CamoufoxEndpoint {
                base_url: "http://localhost:9377".into(),
                api_key: String::new(),
                include_in_auto: true,
            }),
            ..Default::default()
        };
        assert!(!c.camoufox_in_ladder());
    }

    #[cfg(feature = "camoufox")]
    #[test]
    fn camoufox_only_no_cdp_deadline_not_starved() {
        // A camoufox-only deployment (no CDP tiers) with auto-extend on must
        // get a deadline of at least http_timeout + camoufox_timeout, never
        // clamped to the strict default.
        let mut app = AppConfig::default();
        app.request.auto_extend_deadline_for_ladder = true;
        app.renderer.mode = RendererMode::Auto;
        app.renderer.camoufox = Some(CamoufoxEndpoint {
            base_url: "http://localhost:9377".into(),
            api_key: String::new(),
            include_in_auto: true,
        });
        let d = app.effective_deadline_ms(None, None);
        let floor = app.renderer.http_timeout() + app.renderer.camoufox_timeout();
        assert!(
            d >= floor,
            "camoufox-only deadline {d} starved below {floor}"
        );
        // cdp_tier_count must remain 0 — camoufox is REST, never a CDP tier.
        assert_eq!(app.renderer.cdp_tier_count(), 0);
    }

    #[test]
    fn render_js_default_force_js_alias() {
        let cfg: RendererConfig = toml::from_str("force_js = true").unwrap();
        assert_eq!(cfg.render_js_default, Some(true));
    }

    #[test]
    fn render_js_default_direct_field() {
        let cfg: RendererConfig = toml::from_str("render_js_default = false").unwrap();
        assert_eq!(cfg.render_js_default, Some(false));
    }

    #[test]
    fn env_var_renderer_mode_chrome() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_renderer_env();
        unsafe { std::env::set_var("CRW_RENDERER__MODE", "chrome") };
        let cfg = AppConfig::load().unwrap();
        clear_renderer_env();
        assert_eq!(cfg.renderer.mode, RendererMode::Chrome);
    }

    #[test]
    fn env_var_force_js_alias_works() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_renderer_env();
        unsafe { std::env::set_var("CRW_RENDERER__FORCE_JS", "true") };
        let cfg = AppConfig::load().unwrap();
        clear_renderer_env();
        assert_eq!(cfg.renderer.render_js_default, Some(true));
    }

    #[test]
    fn env_var_render_js_default_direct() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_renderer_env();
        unsafe { std::env::set_var("CRW_RENDERER__RENDER_JS_DEFAULT", "true") };
        let cfg = AppConfig::load().unwrap();
        clear_renderer_env();
        assert_eq!(cfg.renderer.render_js_default, Some(true));
    }

    #[test]
    fn request_config_defaults_match_plan() {
        let r = RequestConfig::default();
        assert_eq!(r.deadline_ms_default, 8000);
        assert!(r.auto_extend_deadline_for_ladder);
    }

    #[test]
    fn default_app_config_enables_auto_extend() {
        // Programmatic Default must mirror serde defaults — issue #35.
        let cfg = AppConfig::default();
        assert!(cfg.request.auto_extend_deadline_for_ladder);
        assert_eq!(cfg.request.deadline_ms_default, 8000);
    }

    fn renderer_with_chrome_only(chrome_ms: u64) -> RendererConfig {
        RendererConfig {
            mode: RendererMode::Chrome,
            page_timeout_ms: chrome_ms,
            chrome_timeout_ms: Some(chrome_ms),
            chrome: Some(CdpEndpoint {
                ws_url: "ws://chrome:9222".into(),
            }),
            ..Default::default()
        }
    }

    #[test]
    #[cfg(feature = "cdp")]
    fn min_deadline_full_ladder_chrome_only() {
        // chrome-only mode: http (page_timeout) + chrome + 1 * 28000.
        let r = renderer_with_chrome_only(30_000);
        // page_timeout_ms is set to chrome_ms here, so http_timeout() → 30s.
        assert_eq!(
            r.min_deadline_for_full_ladder_ms(),
            30_000 + 30_000 + 28_000
        );
    }

    #[test]
    #[cfg(feature = "cdp")]
    fn min_deadline_full_ladder_auto_three_tiers() {
        let r = RendererConfig {
            mode: RendererMode::Auto,
            page_timeout_ms: 15_000,
            http_timeout_ms: Some(15_000),
            lightpanda_timeout_ms: Some(2_500),
            chrome_timeout_ms: Some(30_000),
            lightpanda: Some(CdpEndpoint {
                ws_url: "ws://lp:9222".into(),
            }),
            chrome: Some(CdpEndpoint {
                ws_url: "ws://chrome:9222".into(),
            }),
            ..Default::default()
        };
        // http(15) + lp(2.5) + chrome(30) + 2*28 = 47.5 + 56 = 103_500.
        assert_eq!(
            r.min_deadline_for_full_ladder_ms(),
            15_000 + 2_500 + 30_000 + 2 * 28_000
        );
        assert_eq!(r.cdp_tier_count(), 2);
    }

    #[test]
    fn effective_deadline_explicit_bypasses_auto_extend() {
        let mut cfg = AppConfig::default();
        cfg.request.auto_extend_deadline_for_ladder = true;
        cfg.renderer = renderer_with_chrome_only(30_000);
        // Explicit override beats both default and ladder_min.
        assert_eq!(cfg.effective_deadline_ms(Some(5_000), None), 5_000);
        assert_eq!(cfg.effective_deadline_ms(Some(500_000), None), 500_000);
    }

    #[test]
    #[cfg(feature = "cdp")]
    fn effective_deadline_auto_extend_raises_to_ladder_min() {
        let mut cfg = AppConfig::default();
        cfg.request.auto_extend_deadline_for_ladder = true;
        cfg.request.deadline_ms_default = 8_000;
        cfg.renderer = renderer_with_chrome_only(30_000);
        let expected = cfg.renderer.min_deadline_for_full_ladder_ms();
        assert!(expected > 8_000);
        assert_eq!(cfg.effective_deadline_ms(None, None), expected);
    }

    #[test]
    fn effective_deadline_default_wins_when_higher_than_ladder() {
        let mut cfg = AppConfig::default();
        cfg.request.auto_extend_deadline_for_ladder = true;
        cfg.request.deadline_ms_default = 1_000_000;
        cfg.renderer = renderer_with_chrome_only(30_000);
        assert_eq!(cfg.effective_deadline_ms(None, None), 1_000_000);
    }

    #[test]
    fn effective_deadline_auto_extend_disabled_returns_baseline() {
        let mut cfg = AppConfig::default();
        cfg.request.auto_extend_deadline_for_ladder = false;
        cfg.request.deadline_ms_default = 8_000;
        cfg.renderer = renderer_with_chrome_only(30_000);
        assert_eq!(cfg.effective_deadline_ms(None, None), 8_000);
    }

    #[test]
    #[cfg(feature = "cdp")]
    fn effective_deadline_extends_for_long_wait_for() {
        let mut cfg = AppConfig::default();
        cfg.request.auto_extend_deadline_for_ladder = true;
        cfg.request.deadline_ms_default = 8_000;
        cfg.renderer = renderer_with_chrome_only(30_000);
        let base = cfg.renderer.min_deadline_for_full_ladder_ms();
        let tier_count = cfg.renderer.cdp_tier_count() as u64;
        // wait_for = 20000 → per-tier extra = 12000 over SPA_DEFAULT_MS (8000).
        let with_wait = cfg.effective_deadline_ms(None, Some(20_000));
        assert_eq!(with_wait, base + 12_000 * tier_count);
        // wait_for below SPA default → no extra.
        assert_eq!(cfg.effective_deadline_ms(None, Some(2_000)), base);
    }

    #[test]
    fn effective_request_timeout_covers_map_ceiling() {
        let mut cfg = AppConfig::default();
        cfg.request.auto_extend_deadline_for_ladder = true;
        cfg.request.deadline_ms_default = 8_000;
        cfg.renderer = renderer_with_chrome_only(30_000);
        cfg.search.timeout_ms = 15_000;
        cfg.crawler.max_concurrency = 10;
        cfg.search.max_limit = 20;
        cfg.server.request_timeout_secs = 60;
        // Map ceiling 300s + 5s buffer = 305s minimum.
        assert!(cfg.effective_request_timeout_secs() >= 305);
    }

    #[test]
    fn effective_request_timeout_disabled_returns_baseline() {
        let mut cfg = AppConfig::default();
        cfg.request.auto_extend_deadline_for_ladder = false;
        cfg.server.request_timeout_secs = 60;
        assert_eq!(cfg.effective_request_timeout_secs(), 60);
    }

    #[test]
    fn effective_request_timeout_respects_operator_override() {
        let mut cfg = AppConfig::default();
        cfg.request.auto_extend_deadline_for_ladder = true;
        cfg.server.request_timeout_secs = 600; // operator-configured high
        cfg.renderer = renderer_with_chrome_only(30_000);
        // Operator's explicit 600s should win over the auto-computed 305s.
        assert_eq!(cfg.effective_request_timeout_secs(), 600);
    }

    #[test]
    fn effective_request_timeout_search_sequential_batching() {
        // Low concurrency forces ceil(max_limit/conc) batches → larger search_ms.
        let mut cfg = AppConfig::default();
        cfg.request.auto_extend_deadline_for_ladder = true;
        cfg.request.deadline_ms_default = 8_000;
        cfg.renderer = renderer_with_chrome_only(30_000);
        cfg.search.timeout_ms = 15_000;
        cfg.search.max_limit = 20;
        cfg.crawler.max_concurrency = 1;
        cfg.server.request_timeout_secs = 60;
        // The Tower envelope must cover the worst-case implicit scrape with
        // `wait_for` bumped to MAX_WAIT_FOR_MS (60s), because callers can do
        // that without supplying `deadlineMs`. Mirror that in the expected.
        let secs = cfg.effective_request_timeout_secs();
        let scrape_ms = cfg.effective_deadline_ms(None, Some(60_000));
        let expected_search_ms = 15_000 + 20 * scrape_ms;
        let expected_max_ms = scrape_ms.max(expected_search_ms).max(300_000);
        let expected_secs = expected_max_ms.div_ceil(1_000) + 5;
        assert_eq!(secs, 60u64.max(expected_secs));
    }

    #[test]
    #[cfg(not(feature = "cdp"))]
    fn cdp_tier_count_zero_without_cdp_feature() {
        // Even when chrome/lightpanda are configured, a binary built without
        // the `cdp` feature can never construct a JS renderer. The deadline
        // policy must observe that and collapse to HTTP-only behavior.
        let r = RendererConfig {
            mode: RendererMode::Auto,
            page_timeout_ms: 15_000,
            chrome_timeout_ms: Some(30_000),
            chrome: Some(CdpEndpoint {
                ws_url: "ws://chrome:9222".into(),
            }),
            lightpanda: Some(CdpEndpoint {
                ws_url: "ws://lp:9222".into(),
            }),
            ..Default::default()
        };
        assert_eq!(r.cdp_tier_count(), 0);
        // Only the HTTP tier contributes to the ladder budget.
        assert_eq!(r.min_deadline_for_full_ladder_ms(), 15_000);
    }

    #[test]
    fn effective_deadline_skipped_for_http_only_mode() {
        // P2 from codex review: HTTP-only deployments don't suffer the CDP
        // clamping problem (no fetch/challenge/stability overhead). The
        // auto-extension must NOT silently bump their default from 8s to 30s
        // just because page_timeout_ms defaults high.
        let mut cfg = AppConfig::default();
        cfg.request.auto_extend_deadline_for_ladder = true;
        cfg.request.deadline_ms_default = 8_000;
        cfg.renderer = RendererConfig {
            mode: RendererMode::Auto,
            page_timeout_ms: 30_000,
            // No CDP endpoints configured.
            lightpanda: None,
            playwright: None,
            chrome: None,
            ..Default::default()
        };
        assert_eq!(cfg.renderer.cdp_tier_count(), 0);
        assert_eq!(cfg.effective_deadline_ms(None, None), 8_000);
        assert_eq!(cfg.effective_deadline_ms(None, Some(30_000)), 8_000);
    }

    #[test]
    #[cfg(feature = "cdp")]
    fn min_deadline_full_ladder_playwright_only() {
        // Playwright tier contributes one chrome_timeout + one CDP overhead,
        // matching the runtime predicate in `crw-renderer/src/lib.rs`.
        let r = RendererConfig {
            mode: RendererMode::Playwright,
            page_timeout_ms: 15_000,
            http_timeout_ms: Some(15_000),
            chrome_timeout_ms: Some(30_000),
            playwright: Some(CdpEndpoint {
                ws_url: "ws://playwright:9222".into(),
            }),
            ..Default::default()
        };
        assert_eq!(r.cdp_tier_count(), 1);
        // http(15) + chrome-equivalent(30) + 1 * 28 overhead.
        assert_eq!(
            r.min_deadline_for_full_ladder_ms(),
            15_000 + 30_000 + 28_000
        );
    }

    #[test]
    fn renderer_phase_toggles_default_off_or_safe() {
        let r = RendererConfig::default();
        assert!(!r.chrome_intercept_resources);
        assert!(!r.chrome_intercept_stylesheets);
        assert!(r.chrome_host_intercept_disable.is_empty());
        assert_eq!(r.chrome_nav_budget_ms, 12_000);
        assert!(!r.chrome_context_pool_enabled);
        assert!(!r.use_predictor);
    }

    #[test]
    fn crawler_per_host_limiter_defaults() {
        let c = CrawlerConfig::default();
        assert_eq!(c.per_host_min_interval_ms, 0);
        assert_eq!(c.per_host_max_concurrent, 1);
    }

    #[test]
    fn env_var_overrides_toml_defaults() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_renderer_env();
        unsafe {
            std::env::set_var("CRW_SERVER__PORT", "4444");
            std::env::set_var("CRW_RENDERER__LIGHTPANDA__WS_URL", "ws://test:9999/");
        }
        let cfg = AppConfig::load().unwrap();
        clear_renderer_env();

        assert_eq!(cfg.server.port, 4444, "env var should override server.port");
        assert_eq!(
            cfg.renderer.lightpanda.as_ref().unwrap().ws_url,
            "ws://test:9999/",
            "env var should override renderer.lightpanda.ws_url"
        );
    }

    #[test]
    fn crawler_proxy_empty_env_normalizes_to_none() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_renderer_env();
        // Isolate from any developer ~/.config/crw/config.toml and stray CRW_CONFIG.
        let tmp = std::env::temp_dir().join(format!("crw-proxy-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        unsafe {
            std::env::set_var("CRW_USER_CONFIG_DIR", &tmp);
            std::env::remove_var("CRW_CONFIG");
        }

        let load = || {
            // CRW_CONFIG and the user config dir are pinned for the whole test.
            AppConfig::load().unwrap().crawler.proxy
        };

        // (1) absent -> None (serde default).
        unsafe { std::env::remove_var("CRW_CRAWLER__PROXY") };
        assert_eq!(load(), None, "absent proxy env should be None");

        // (2) present-but-empty -> None (the issue #154 case).
        unsafe { std::env::set_var("CRW_CRAWLER__PROXY", "") };
        assert_eq!(load(), None, "empty proxy env should normalize to None");

        // (3) whitespace-only -> None.
        unsafe { std::env::set_var("CRW_CRAWLER__PROXY", "   ") };
        assert_eq!(
            load(),
            None,
            "whitespace proxy env should normalize to None"
        );

        // (4) a real value -> Some (trimmed).
        unsafe { std::env::set_var("CRW_CRAWLER__PROXY", "  http://proxy:8080  ") };
        assert_eq!(
            load(),
            Some("http://proxy:8080".to_string()),
            "valid proxy env should be preserved and trimmed"
        );

        unsafe {
            std::env::remove_var("CRW_CRAWLER__PROXY");
            std::env::remove_var("CRW_USER_CONFIG_DIR");
        }
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn user_config_path_honors_override_env() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = std::env::temp_dir().join(format!("crw-cfg-test-{}", std::process::id()));
        unsafe {
            std::env::set_var("CRW_USER_CONFIG_DIR", &tmp);
        }
        let p = user_config_path().unwrap();
        unsafe {
            std::env::remove_var("CRW_USER_CONFIG_DIR");
        }
        assert_eq!(p, tmp.join("config.toml"));
    }

    #[test]
    fn user_config_file_is_picked_up_by_load() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_renderer_env();
        let tmp = std::env::temp_dir().join(format!("crw-load-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let cfg_path = tmp.join("config.toml");
        std::fs::write(
            &cfg_path,
            r#"
[client]
api_url = "https://api.example.com"
api_key = "test-key-123"

[search]
searxng_url = "http://localhost:9999"

[extraction.llm]
provider = "deepseek"
api_key = "sk-test"
model = "deepseek-chat"
"#,
        )
        .unwrap();

        unsafe {
            std::env::set_var("CRW_USER_CONFIG_DIR", &tmp);
        }
        let cfg = AppConfig::load().unwrap();
        unsafe {
            std::env::remove_var("CRW_USER_CONFIG_DIR");
        }
        std::fs::remove_dir_all(&tmp).ok();

        assert_eq!(
            cfg.client.api_url.as_deref(),
            Some("https://api.example.com")
        );
        assert_eq!(cfg.client.api_key.as_deref(), Some("test-key-123"));
        assert_eq!(
            cfg.search.searxng_url.as_deref(),
            Some("http://localhost:9999")
        );
        let llm = cfg.extraction.llm.expect("llm config present");
        assert_eq!(llm.provider, "deepseek");
        assert_eq!(llm.api_key, "sk-test");
    }

    #[test]
    fn env_var_beats_user_config() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_renderer_env();
        let tmp = std::env::temp_dir().join(format!("crw-prec-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("config.toml"),
            r#"
[search]
searxng_url = "http://from-file:8080"
"#,
        )
        .unwrap();

        unsafe {
            std::env::set_var("CRW_USER_CONFIG_DIR", &tmp);
            std::env::set_var("CRW_SEARCH__SEARXNG_URL", "http://from-env:8080");
        }
        let cfg = AppConfig::load().unwrap();
        unsafe {
            std::env::remove_var("CRW_USER_CONFIG_DIR");
            std::env::remove_var("CRW_SEARCH__SEARXNG_URL");
        }
        std::fs::remove_dir_all(&tmp).ok();

        assert_eq!(
            cfg.search.searxng_url.as_deref(),
            Some("http://from-env:8080"),
            "env var must win over user config file"
        );
    }

    #[test]
    fn effective_proxy_credentials_appends_country_suffix() {
        let cfg = RendererConfig {
            proxy_base_user: Some("abc".into()),
            proxy_base_pass: Some("pw".into()),
            proxy_default_country: Some("de".into()),
            ..Default::default()
        };
        let (u, p) = cfg.effective_proxy_credentials(Some("us")).unwrap();
        assert_eq!(u, "abc__cr.us");
        assert_eq!(p, "pw");
        // Per-request wins over default.
        let (u, _) = cfg.effective_proxy_credentials(Some("GB")).unwrap();
        assert_eq!(u, "abc__cr.gb", "uppercase input is normalized");
        // Default country used when per-request omits it.
        let (u, _) = cfg.effective_proxy_credentials(None).unwrap();
        assert_eq!(u, "abc__cr.de");
    }

    #[test]
    fn effective_proxy_credentials_invalid_country_uses_global_pool() {
        let cfg = RendererConfig {
            proxy_base_user: Some("abc".into()),
            proxy_base_pass: Some("pw".into()),
            ..Default::default()
        };
        // 3-letter ISO code → rejected, no suffix (global pool).
        let (u, _) = cfg.effective_proxy_credentials(Some("usa")).unwrap();
        assert_eq!(u, "abc");
        // Digits → rejected.
        let (u, _) = cfg.effective_proxy_credentials(Some("u1")).unwrap();
        assert_eq!(u, "abc");
        // Empty string after trim → rejected.
        let (u, _) = cfg.effective_proxy_credentials(Some("  ")).unwrap();
        assert_eq!(u, "abc");
    }

    #[test]
    fn effective_proxy_credentials_no_base_returns_none() {
        let cfg = RendererConfig::default();
        assert!(cfg.effective_proxy_credentials(Some("us")).is_none());

        let only_user = RendererConfig {
            proxy_base_user: Some("abc".into()),
            ..Default::default()
        };
        assert!(only_user.effective_proxy_credentials(Some("us")).is_none());
    }
}
