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
    /// Additive on top of `DEFAULT_TRACKING_PARAMS`.
    #[serde(default)]
    pub extra_tracking_params: Vec<String>,
    /// Additive on top of `DEFAULT_ACTION_PARAMS`.
    #[serde(default)]
    pub extra_action_params: Vec<String>,
    /// Additive on top of `ALWAYS_PRESERVE`.
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
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            searxng_url: None,
            timeout_ms: default_search_timeout_ms(),
            default_limit: default_search_limit(),
            max_limit: default_search_max_limit(),
            research_engines: default_research_engines(),
            github_engines: default_github_engines(),
        }
    }
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
    /// reuse-with-clearing. See plan Phase 4.
    #[serde(default)]
    pub chrome_context_pool_enabled: bool,
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
    /// Run the classifier on every fetch result. Cheap; default on.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// When the classifier returns a non-`None` signal, advance to the next
    /// engine tier (requires `escalation.enabled`).
    #[serde(default)]
    pub escalate_on_signal: bool,
}

impl Default for AntibotConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            escalate_on_signal: false,
        }
    }
}

fn default_chrome_nav_budget_ms() -> u64 {
    12_000
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
            render_js_default: None,
            lightpanda: None,
            playwright: None,
            chrome: None,
            chrome_intercept_resources: false,
            chrome_intercept_stylesheets: false,
            chrome_host_intercept_disable: Vec::new(),
            chrome_nav_budget_ms: default_chrome_nav_budget_ms(),
            chrome_context_pool_enabled: false,
            use_predictor: false,
            escalation: EscalationConfig::default(),
            antibot: AntibotConfig::default(),
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
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:133.0) Gecko/20100101 Firefox/133.0",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_7_2) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.2 Safari/605.1.15",
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
    /// (e.g. "http://proxy:8080" or "socks5://user:pass@proxy:1080").
    #[serde(default)]
    pub proxy: Option<String>,
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
     (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
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
}

fn default_http_retry_threshold() -> usize {
    100
}

fn default_lightpanda_retry_threshold() -> usize {
    2000
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

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AuthConfig {
    #[serde(default)]
    pub api_keys: Vec<String>,
}

impl AppConfig {
    /// Load config from config.default.toml + environment variable overrides.
    /// Env vars use `CRW_` prefix, `__` as separator. E.g. `CRW_SERVER__PORT=8080`.
    pub fn load() -> Result<Self, config::ConfigError> {
        let mut builder = config::Config::builder()
            .add_source(config::File::with_name("config.default").required(false));

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
        if self.renderer.cdp_tier_count() == 0 {
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
}
