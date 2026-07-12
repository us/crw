//! HTTP and headless-browser rendering engine for the CRW web scraper.
//!
//! Provides a [`FallbackRenderer`] that fetches pages via plain HTTP and optionally
//! re-renders them through a CDP-based headless browser when SPA content is detected.
//!
//! - [`http_only`] — Simple HTTP fetcher using `reqwest`
//! - [`detector`] — Heuristic SPA shell detection (empty body, framework markers)
//! - `cdp` — Chrome DevTools Protocol renderer (LightPanda, Playwright, Chrome) *(requires `cdp` feature)*
//! - [`traits`] — [`PageFetcher`] trait for pluggable backends
//!
//! # Feature flags
//!
//! | Flag  | Description |
//! |-------|-------------|
//! | `cdp` | Enables CDP WebSocket rendering via `tokio-tungstenite` |
//!
//! # Example
//!
//! ```rust,no_run
//! use crw_core::config::RendererConfig;
//! use crw_renderer::FallbackRenderer;
//! use std::collections::HashMap;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! use crw_core::config::StealthConfig;
//! let config = RendererConfig::default();
//! let stealth = StealthConfig::default();
//! let renderer = FallbackRenderer::new(&config, "crw/0.1", None, &stealth)?;
//! let deadline = crw_core::Deadline::from_request_ms(8000);
//! let result = renderer.fetch("https://example.com", &HashMap::new(), None, None, None, deadline).await?;
//! println!("status: {}", result.status_code);
//! # Ok(())
//! # }
//! ```

pub mod blocklist;
pub mod breaker;
#[cfg(feature = "auto-browser")]
pub mod browser;
#[cfg(feature = "cdp")]
pub mod browser_pool;
#[cfg(feature = "camoufox")]
pub mod camoufox;
#[cfg(feature = "cdp")]
pub mod cdp;
#[cfg(feature = "cdp")]
pub mod cdp_conn;
pub mod detector;
#[cfg(feature = "cdp")]
pub mod health_telemetry;
pub mod host_limiter;
pub mod http_only;
pub mod preference;
pub mod traits;

use crate::breaker::{
    AttemptContext, BreakerOutcome, BreakerRegistry, Permit, ProbeGuard, classify_outcome,
};
use crate::preference::HostPreferences;
use crw_core::config::{BUILTIN_UA_POOL, RendererConfig, RendererMode, StealthConfig};
use crw_core::error::{CrwError, CrwResult};
use crw_core::metrics::metrics;
use crw_core::types::{
    FailoverErrorKind, FetchResult, RenderDecision, RendererKind, resolve_render_js,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use traits::PageFetcher;

tokio::task_local! {
    /// Per-request country code (ISO 3166-1 alpha-2, lowercase) for the
    /// chrome_proxy tier's CDP auth pump. Set by `FallbackRenderer::fetch`
    /// when a `ScrapeRequest.country` is present; read in `cdp.rs` while
    /// composing DataImpulse credentials. Task-local so child tasks
    /// spawned by the pool inherit it without trait-signature churn.
    pub static REQUEST_COUNTRY: Option<String>;
}

tokio::task_local! {
    /// Resolved proxy entry for the current request, picked from the active
    /// rotator by host. Set by the scrape/crawl entry points (via
    /// [`FallbackRenderer::pick_proxy`]); read in `cdp.rs` to drive the
    /// per-request Chrome `proxyServer` (a fresh proxied browser context) and
    /// the `Fetch.authRequired` pump. `None` = no proxy → existing behaviour.
    pub static REQUEST_PROXY: Option<Arc<crw_core::ProxyEntry>>;
}

/// Per-request screenshot capture parameters. Carried via a task-local rather
/// than the `PageFetcher::fetch` signature (mirrors [`REQUEST_PROXY`]) so the
/// trait + its ~30 call sites stay untouched. `Some` ⇒ capture a PNG via CDP
/// `Page.captureScreenshot` after the wait window; `None` ⇒ no screenshot.
#[derive(Debug, Clone, Copy)]
pub struct ScreenshotReq {
    /// Capture the full scrollable page (`captureBeyondViewport`) vs. just the
    /// current viewport.
    pub full_page: bool,
}

tokio::task_local! {
    /// Resolved screenshot request for the current scrape. Set by the
    /// scrape/crawl entry point ([`crw_crawl::single::scrape_url`]) when
    /// `formats` contains `Screenshot`; read in `cdp.rs` to drive
    /// `Page.captureScreenshot` and in [`FallbackRenderer::fetch`] to force the
    /// vanilla-Chrome CDP path. `None` = no screenshot → existing behaviour.
    pub static REQUEST_SCREENSHOT: Option<ScreenshotReq>;
}

/// Interactive render-slot reserve for a Chrome pool of `pool_size` (the B
/// reserved lane). About a quarter of the pool, floored at 1 whenever there are
/// ≥2 slots so even small (2–3 slot) self-hosted pools keep interactive render
/// isolation; only a 1-slot pool disables it (a reserve there would starve
/// batch). Always kept below `pool_size` so the batch gate stays ≥1.
pub fn render_reserve(pool_size: usize) -> usize {
    if pool_size <= 1 {
        0
    } else {
        (pool_size / 4).max(1).min(pool_size - 1)
    }
}

/// Whether the named renderer tier can capture a screenshot.
///
/// Screenshot capture is CDP `Page.captureScreenshot` on vanilla Chrome
/// (`chrome`, `chrome_proxy`, `playwright`). LightPanda's CdpRenderer returns a
/// ~30-byte stub and Camoufox is an HTTP sidecar that doesn't speak CDP —
/// neither can capture.
///
/// SINGLE SOURCE OF TRUTH: used both by the request-time renderer filter in
/// `FallbackRenderer::fetch_with_js` and by
/// [`FallbackRenderer::supports_screenshot`] (which `/v1/capabilities` reports),
/// so the advertised capability and the runtime behaviour cannot drift apart.
pub fn renderer_can_screenshot(name: &str) -> bool {
    name != "lightpanda" && name != "camoufox"
}

/// Whether a screenshot was requested for the current task (reads the
/// [`REQUEST_SCREENSHOT`] task-local). `false` when unset / outside a scope.
pub fn screenshot_requested() -> bool {
    REQUEST_SCREENSHOT
        .try_with(|s| s.is_some())
        .unwrap_or(false)
}

/// The resolved screenshot params for the current task, if any.
pub fn current_screenshot_req() -> Option<ScreenshotReq> {
    REQUEST_SCREENSHOT.try_with(|s| *s).ok().flatten()
}

/// Map a renderer's name string to the closed `RendererKind` enum.
/// Returns `None` for unknown names (e.g. "playwright" — treated as a
/// JS renderer but not tracked in metrics/preferences).
fn renderer_kind_for(name: &str) -> Option<RendererKind> {
    match name {
        "http" | "http_only_fallback" => Some(RendererKind::Http),
        "lightpanda" => Some(RendererKind::Lightpanda),
        "chrome" => Some(RendererKind::Chrome),
        "chrome_proxy" => Some(RendererKind::ChromeProxy),
        "camoufox" => Some(RendererKind::Camoufox),
        _ => None,
    }
}

/// Classify a renderer-side error into a `FailoverErrorKind` for the
/// preference learner. Match on `CrwError` variants (not error strings),
/// so renaming or rewording the human-readable message can't silently
/// reclassify failures and over-promote hosts.
///
/// Only LightPanda-specific failures drive promotion (see
/// [`FailoverErrorKind::counts_for_promotion`]); transport / unreachable
/// errors stay in `NetworkError` so a flaky upstream doesn't push hosts
/// to Chrome.
fn classify_renderer_error(err: &CrwError) -> FailoverErrorKind {
    match err {
        CrwError::Timeout(_) => FailoverErrorKind::LightpandaTimeout,
        CrwError::TargetUnreachable(_) => FailoverErrorKind::NetworkError,
        CrwError::HttpError(_) => FailoverErrorKind::NetworkError,
        // RendererError covers WS disconnects, CDP frame errors, render
        // pipeline crashes — these are LightPanda-attributable.
        CrwError::RendererError(_) => FailoverErrorKind::LightpandaCrash,
        _ => FailoverErrorKind::Other,
    }
}

/// Build a per-tier timeout map from the renderer config. Used by the
/// breaker layer for pre-flight skip and clamp detection.
fn tier_timeouts_from(
    config: &RendererConfig,
) -> std::collections::HashMap<RendererKind, std::time::Duration> {
    let mut m = std::collections::HashMap::new();
    m.insert(
        RendererKind::Http,
        std::time::Duration::from_millis(config.http_timeout()),
    );
    m.insert(
        RendererKind::Lightpanda,
        std::time::Duration::from_millis(config.lightpanda_timeout()),
    );
    m.insert(
        RendererKind::Chrome,
        std::time::Duration::from_millis(config.chrome_timeout()),
    );
    m.insert(
        RendererKind::ChromeProxy,
        std::time::Duration::from_millis(config.chrome_proxy_timeout()),
    );
    // Unconditional: `camoufox_timeout()` exists regardless of feature. The map
    // entry is consulted only when a camoufox renderer is actually in the pool,
    // so an unused entry in lean builds is harmless and keeps this function
    // feature-free.
    m.insert(
        RendererKind::Camoufox,
        std::time::Duration::from_millis(config.camoufox_timeout()),
    );
    m
}

/// Credit cost per fetched page. Flat 1 for every renderer: the SaaS bills 1
/// credit per scrape regardless of renderer, and `data.credit_cost` is the
/// field docs tell users to audit their charge against — so it must equal that
/// charge. ponytail: per-renderer pricing removed; re-add a `match kind` here
/// (e.g. `Chrome => 2`) if a renderer ever needs to cost more than the base.
fn credit_for(_kind: RendererKind) -> u32 {
    1
}

/// Stamp `render_decision` and `credit_cost` for an HTTP-only result.
/// `requested_renderer` is taken into account: if the user explicitly
/// pinned `"http"` we mark it as `UserPinned`, otherwise `AutoDefault`.
fn stamp_http_decision(result: &mut FetchResult, requested_renderer: Option<&str>) {
    if result.render_decision.is_some() {
        return;
    }
    let kind = RendererKind::Http;
    result.credit_cost = credit_for(kind);
    result.render_decision = Some(match requested_renderer {
        Some("http") => RenderDecision::UserPinned { renderer: kind },
        _ => RenderDecision::AutoDefault { chosen: kind },
    });
    // Mirror the JS-renderer metric so dashboards see HTTP routing too.
    metrics()
        .render_route_decision_total
        .with_label_values(&[kind.as_str(), "success"])
        .inc();
}

/// Extract the host from a URL string, returning an empty string on failure.
fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_default()
}

/// Pick a user-agent: rotate from stealth pool when stealth is enabled.
fn pick_ua<'a>(default_ua: &'a str, stealth: &'a StealthConfig) -> String {
    if stealth.enabled {
        let pool: &[&str] = if stealth.user_agents.is_empty() {
            BUILTIN_UA_POOL
        } else {
            // Safe: user_agents is non-empty in this branch.
            return stealth.user_agents[rand::random_range(0..stealth.user_agents.len())].clone();
        };
        pool[rand::random_range(0..pool.len())].to_string()
    } else {
        default_ua.to_string()
    }
}

/// Pure classification of a JS-renderer result (no side-effects). Produced by
/// `FallbackRenderer::classify_js_attempt`; consumed by the serial loop and the
/// conditional hedge to apply the identical accept-gate.
#[allow(dead_code)] // full classification kept for completeness; hedge uses a subset
struct JsAttemptClass {
    text_len: usize,
    is_placeholder: bool,
    failed_render: Option<detector::FailedRenderReason>,
    is_bot_wall: bool,
    vendor_block: Option<&'static str>,
    is_status_blocked: bool,
    antibot: crw_extract::antibot::AntibotResult,
    antibot_blocked: bool,
    /// Egress-recoverable hard-block (drives the gated chrome_proxy recovery arm).
    hard_block: bool,
    /// Passes the success accept-gate (return as-is, don't escalate).
    acceptable: bool,
}

/// Result of the conditional hedge (race lightpanda+chrome).
enum HedgeOutcome {
    /// A tier passed the accept-gate — return as-is (terminal).
    Accepted(FetchResult),
    /// Both tiers thin/failed — best-thin result + whether a hard-block was seen
    /// (so the caller can fire the gated auto-egress recovery arm). Mirrors the
    /// serial loop's `thin_result` + `saw_hard_block` fall-through.
    Thin(FetchResult, bool),
}

/// Did this renderer error come from failing to reach/navigate the ORIGIN, as
/// opposed to a fault on our side (CDP pool exhausted, browser discovery failed,
/// a pinned renderer that does not exist)?
///
/// Only used to decide whether an unreachable origin should outrank a JS-tier error
/// when both fail. Getting it wrong in the permissive direction (treating our fault as
/// the origin's) would blame the caller for our outage, so the match is deliberately
/// narrow.
///
/// `Timeout` is NOT included: a JS-tier timeout is just as likely to be a local CDP
/// websocket/command timeout as a slow origin, and it already maps to 504 on its own,
/// which is the honest answer either way.
fn is_origin_navigation_failure(e: &CrwError) -> bool {
    match e {
        CrwError::TargetUnreachable(_) => true,
        // Chrome/LightPanda report a failure to reach the origin as a navigation error
        // with a `net::ERR_*` code. Internal faults (pool exhausted, CDP discovery)
        // carry different messages and keep their own error.
        CrwError::RendererError(msg) => {
            let m = msg.to_ascii_lowercase();
            m.contains("navigation failed") || m.contains("net::err_")
        }
        _ => false,
    }
}

/// Minimum remaining request budget for a network attempt to be worth making.
/// Below this a CDP tier cannot complete its handshake and returns a fabricated
/// `Timeout after Nms` (single-digit N) while still consuming a pool slot.
/// Guards the main ladder loop, the hedge dispatch, the breaker leak-through arm,
/// and the HTTP tier's proxy retry (`http_only`).
pub(crate) const MIN_TIER_BUDGET: Duration = Duration::from_millis(500);

/// Composite renderer that tries multiple backends in order.
pub struct FallbackRenderer {
    http: Arc<dyn PageFetcher>,
    js_renderers: Vec<Arc<dyn PageFetcher>>,
    /// Global default for `render_js` when a request doesn't specify one.
    render_js_default: Option<bool>,
    /// Phase 0 (latency-qn): emit per-fetch structured timing for bench runs.
    latency_breakdown: bool,
    /// Phase 2 (latency-qn): gate chrome_proxy as a hard-block-only recovery arm
    /// (removed from the normal ladder) instead of an always-on tier.
    auto_egress_escalation: bool,
    /// latency-qn: conditional hedge — race lightpanda+chrome concurrently.
    chrome_hedge: bool,
    /// Headroom gate for the hedge: bounds concurrent hedges to pool_size/2 so the
    /// 2-contexts-per-request hedge can never deadlock the context pool. Acquired
    /// with `try_acquire` (no permit → serial fallback; blocking would defeat the
    /// latency win).
    hedge_sem: Arc<tokio::sync::Semaphore>,
    /// Per-host renderer preference learning (auto-mode only).
    preferences: Arc<HostPreferences>,
    /// Per-host + global circuit breakers per renderer.
    breakers: Arc<BreakerRegistry>,
    /// Per-tier configured timeouts (Duration). Used by the breaker layer
    /// for pre-flight deadline-skip and clamp detection in
    /// `AttemptContext::capture`.
    tier_timeouts: std::collections::HashMap<RendererKind, std::time::Duration>,
    /// Process-wide per-eTLD+1 rate (req/sec). `0.0` disables the interval
    /// floor; the concurrency cap below still applies. Configured via
    /// [`Self::with_host_limits`].
    requests_per_second: f64,
    /// Process-wide per-eTLD+1 in-flight cap for batch/crawl. `1` enforces strict
    /// politeness. Interactive gets `per_host_interactive_reserve` extra slots.
    per_host_max_concurrent: u32,
    /// Extra per-host in-flight slots reserved for interactive traffic (the A
    /// reserved lane). Total per-host in-flight is bounded by
    /// `per_host_max_concurrent + this`.
    per_host_interactive_reserve: u32,
    /// Anti-bot classifier policy. Drives the in-loop `classify()` call that
    /// decides whether a 200-status block page is a soft failure (escalate
    /// toward `chrome_proxy`) or a genuine success.
    antibot: crw_core::config::AntibotConfig,
    /// Active proxy rotator. Drives the HTTP fetcher pool and (with the `cdp`
    /// feature) per-request CDP `proxyServer` selection. `None` = no proxy
    /// configured → direct connections, byte-identical to prior behavior.
    proxy_rotator: Option<Arc<crw_core::ProxyRotator>>,
    /// Saved HTTP-fetcher construction inputs so a per-request proxied client
    /// can be built on demand (when `REQUEST_PROXY` is set) without re-picking.
    http_ua: String,
    http_inject_stealth: bool,
    http_timeout_ms: u64,
    /// Warm per-proxy HTTP fetchers keyed by `ProxyEntry::raw()`, so repeated
    /// requests to the same proxy reuse a connection pool instead of rebuilding
    /// a client each time. Bounded — cleared past a cap to avoid unbounded
    /// growth under arbitrary BYOP proxies.
    proxy_client_cache: std::sync::Mutex<std::collections::HashMap<String, Arc<dyn PageFetcher>>>,
    /// Chrome browser-context pool handle for graceful drain on shutdown.
    /// `None` when the pool is disabled or the chrome tier isn't configured.
    #[cfg(feature = "cdp")]
    chrome_pool: Option<Arc<browser_pool::BrowserContextPool<cdp_conn::CdpConnection>>>,
    /// Whether the (constructed) camoufox tier participates in the auto ladder
    /// for this instance's mode. Drives the non-pinned pool filter in
    /// `fetch_with_js`: when false, a configured camoufox renderer is reachable
    /// only by an explicit `renderer = "camoufox"` pin, never the auto chain.
    #[cfg(feature = "camoufox")]
    camoufox_in_auto: bool,
}

impl std::fmt::Debug for FallbackRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FallbackRenderer")
            .field("http", &self.http.name())
            .field(
                "js_renderers",
                &self
                    .js_renderers
                    .iter()
                    .map(|r| r.name())
                    .collect::<Vec<_>>(),
            )
            .field("render_js_default", &self.render_js_default)
            .finish()
    }
}

impl FallbackRenderer {
    pub fn new(
        config: &RendererConfig,
        user_agent: &str,
        proxy: Option<&str>,
        stealth: &StealthConfig,
    ) -> CrwResult<Self> {
        let effective_ua = pick_ua(user_agent, stealth);
        let inject_headers = stealth.enabled && stealth.inject_headers;
        let http_timeout_ms = config.http_timeout();
        // Fail closed: a malformed single `proxy` (e.g. CLI `--proxy htp://...`)
        // is a hard error, never a silent direct connection (real-IP leak).
        if let Some(p) = proxy {
            crw_core::ProxyEntry::parse(p).map_err(CrwError::ConfigError)?;
        }
        let http = Arc::new(http_only::HttpFetcher::with_timeout(
            &effective_ua,
            proxy,
            inject_headers,
            std::time::Duration::from_millis(http_timeout_ms),
        )) as Arc<dyn PageFetcher>;

        // A pinned backend (Lightpanda/Chrome/Playwright) must have CDP compiled in
        // AND its matching endpoint configured. `Auto` and `None` remain functional
        // without CDP — they just won't spawn any JS renderer.
        #[cfg(not(feature = "cdp"))]
        if matches!(
            config.mode,
            RendererMode::Lightpanda | RendererMode::Chrome | RendererMode::Playwright
        ) {
            return Err(CrwError::ConfigError(format!(
                "renderer.mode = {:?} requires the 'cdp' feature, but this build was \
                 compiled without it. Rebuild with --features cdp or set mode = \"auto\"/\"none\".",
                config.mode
            )));
        }

        // Camoufox is REST, not CDP — it requires the `camoufox` feature
        // independently of `cdp`. Separate top-level guard (never nested in the
        // cdp block above) so a camoufox-less build rejects the pin cleanly.
        #[cfg(not(feature = "camoufox"))]
        if matches!(config.mode, RendererMode::Camoufox) {
            return Err(CrwError::ConfigError(
                "renderer.mode = \"camoufox\" requires the 'camoufox' feature, but this build \
                 was compiled without it. Rebuild with --features camoufox or set mode = \
                 \"auto\"/\"none\"."
                    .into(),
            ));
        }

        #[allow(unused_mut)]
        let mut js_renderers: Vec<Arc<dyn PageFetcher>> = Vec::new();

        if matches!(config.mode, RendererMode::None) {
            if config.render_js_default == Some(true) {
                tracing::warn!(
                    "render_js_default=true has no effect with mode=none; \
                     requests will fall back to HTTP via http_only_fallback"
                );
            }
            return Ok(Self {
                http,
                js_renderers,
                render_js_default: config.render_js_default,
                latency_breakdown: config.latency_breakdown,
                auto_egress_escalation: config.auto_egress_escalation,
                chrome_hedge: config.chrome_hedge,
                hedge_sem: Arc::new(tokio::sync::Semaphore::new((config.pool_size / 2).max(1))),
                preferences: Arc::new(HostPreferences::with_defaults()),
                breakers: Arc::new(BreakerRegistry::with_defaults()),
                tier_timeouts: tier_timeouts_from(config),
                requests_per_second: 0.0,
                per_host_max_concurrent: 1,
                per_host_interactive_reserve: 1,
                antibot: config.antibot.clone(),
                proxy_rotator: None,
                http_ua: effective_ua.clone(),
                http_inject_stealth: inject_headers,
                http_timeout_ms,
                proxy_client_cache: std::sync::Mutex::new(std::collections::HashMap::new()),
                #[cfg(feature = "cdp")]
                chrome_pool: None,
                // mode=none constructs no renderers, so camoufox is never in
                // the (empty) ladder.
                #[cfg(feature = "camoufox")]
                camoufox_in_auto: false,
            });
        }

        #[cfg(feature = "cdp")]
        let mut chrome_pool: Option<
            Arc<browser_pool::BrowserContextPool<cdp_conn::CdpConnection>>,
        > = None;

        #[cfg(feature = "cdp")]
        {
            let want = |m: RendererMode| -> bool {
                matches!(config.mode, RendererMode::Auto) || config.mode == m
            };

            if want(RendererMode::Lightpanda) {
                if let Some(lp) = &config.lightpanda {
                    js_renderers.push(Arc::new(
                        cdp::CdpRenderer::new(
                            "lightpanda",
                            &lp.ws_url,
                            config.lightpanda_timeout(),
                            config.pool_size,
                        )
                        .with_user_agent(&effective_ua),
                    ));
                } else if matches!(config.mode, RendererMode::Lightpanda) {
                    return Err(CrwError::ConfigError(
                        "renderer.mode = \"lightpanda\" but [renderer.lightpanda] ws_url is not \
                         configured"
                            .into(),
                    ));
                }
            }
            if want(RendererMode::Playwright) {
                if let Some(pw) = &config.playwright {
                    // Playwright is treated as a "chrome-equivalent" tier —
                    // same timeout budget, same kind of work.
                    js_renderers.push(Arc::new(
                        cdp::CdpRenderer::new(
                            "playwright",
                            &pw.ws_url,
                            config.chrome_timeout(),
                            config.pool_size,
                        )
                        .with_user_agent(&effective_ua),
                    ));
                } else if matches!(config.mode, RendererMode::Playwright) {
                    return Err(CrwError::ConfigError(
                        "renderer.mode = \"playwright\" but [renderer.playwright] ws_url is not \
                         configured"
                            .into(),
                    ));
                }
            }
            if want(RendererMode::Chrome) {
                if let Some(ch) = &config.chrome {
                    let blocklist = blocklist::Blocklist::defaults()
                        .with_stylesheets(config.chrome_intercept_stylesheets);
                    let mut renderer = cdp::CdpRenderer::new(
                        "chrome",
                        &ch.ws_url,
                        config.chrome_timeout(),
                        config.pool_size,
                    )
                    .with_user_agent(&effective_ua)
                    .with_nav_budget(config.chrome_nav_budget_ms)
                    .with_challenge_retries(
                        config
                            .chrome_challenge_max_retries
                            .unwrap_or(cdp::CHALLENGE_MAX_RETRIES),
                    )
                    .with_spa_selector_max(
                        config
                            .chrome_spa_selector_max_ms
                            .unwrap_or(cdp::SPA_SELECTOR_MAX_MS),
                    )
                    .with_fast_ready(config.chrome_fast_ready)
                    .with_interception(
                        config.chrome_intercept_resources,
                        blocklist,
                        config.chrome_host_intercept_disable.clone(),
                    );

                    // Browser-context pool: gated off on browserless v2 in v1
                    // per plan §"Out of scope". The backend is set explicitly
                    // in config; never URL-sniffed.
                    if config.chrome_context_pool_enabled {
                        match config.chrome_backend {
                            crw_core::config::ChromeBackend::Vanilla => {
                                let pcfg = &config.chrome_pool;
                                let size = pcfg.size.unwrap_or_else(|| {
                                    let n = std::thread::available_parallelism()
                                        .map(|p| p.get())
                                        .unwrap_or(2);
                                    std::cmp::max(2, n / 2)
                                });
                                renderer = renderer.with_pool(browser_pool::PoolCfg {
                                    size,
                                    recycle_after_navs: pcfg.recycle_after_navs,
                                    idle_timeout: std::time::Duration::from_secs(
                                        pcfg.idle_timeout_secs,
                                    ),
                                    health_check_after: std::time::Duration::from_secs(
                                        pcfg.health_check_secs,
                                    ),
                                    shutdown_drain: std::time::Duration::from_secs(
                                        pcfg.shutdown_drain_secs,
                                    ),
                                    close_target_timeout: std::time::Duration::from_secs(2),
                                    dispose_ctx_timeout: std::time::Duration::from_secs(1),
                                    create_ctx_timeout: std::time::Duration::from_secs(1),
                                });
                                tracing::info!(
                                    pool_size = size,
                                    "chrome browser-context pool enabled"
                                );
                            }
                            crw_core::config::ChromeBackend::Browserless => {
                                tracing::warn!(
                                    "chrome_context_pool_enabled = true but \
                                     chrome_backend = browserless — pool unsupported on \
                                     this backend in v1, falling back to legacy path"
                                );
                            }
                        }
                    }
                    chrome_pool = renderer.pool();
                    js_renderers.push(Arc::new(renderer));
                } else if matches!(config.mode, RendererMode::Chrome) {
                    return Err(CrwError::ConfigError(
                        "renderer.mode = \"chrome\" but [renderer.chrome] ws_url is not configured"
                            .into(),
                    ));
                }
                // Residential-proxy Chrome tier: opt-in 4th renderer. Pushed
                // after `chrome` so the existing in-request fallback loop
                // (`for renderer in renderers` in fetch_with_js) tries Chrome
                // direct first and falls through to chrome_proxy on failure.
                // Skipped when [renderer.chrome_proxy] is unset OR when
                // `ws_url` is empty (docker-compose passes empty env vars
                // even when --profile proxy is inactive).
                if let Some(cp) = config
                    .chrome_proxy
                    .as_ref()
                    .filter(|c| !c.ws_url.trim().is_empty())
                {
                    let blocklist = blocklist::Blocklist::defaults()
                        .with_stylesheets(config.chrome_intercept_stylesheets);
                    let mut renderer = cdp::CdpRenderer::new(
                        "chrome_proxy",
                        &cp.ws_url,
                        config.chrome_proxy_timeout(),
                        config.pool_size,
                    )
                    .with_user_agent(&effective_ua)
                    .with_nav_budget(config.chrome_nav_budget_ms)
                    .with_challenge_retries(
                        config
                            .chrome_challenge_max_retries
                            .unwrap_or(cdp::CHALLENGE_MAX_RETRIES),
                    )
                    .with_spa_selector_max(
                        config
                            .chrome_spa_selector_max_ms
                            .unwrap_or(cdp::SPA_SELECTOR_MAX_MS),
                    )
                    .with_fast_ready(config.chrome_fast_ready)
                    .with_interception(
                        config.chrome_intercept_resources,
                        blocklist,
                        config.chrome_host_intercept_disable.clone(),
                    );
                    // Wire DataImpulse base creds when configured. The renderer
                    // composes `{base_user}__cr.{country}` per request and replies
                    // to Chrome's `Fetch.authRequired` via CDP — replacing the
                    // removed gost forwarder.
                    if let (Some(u), Some(p)) = (&config.proxy_base_user, &config.proxy_base_pass) {
                        renderer = renderer.with_proxy_auth_base(
                            u.clone(),
                            p.clone(),
                            config.proxy_default_country.clone(),
                        );
                    }
                    tracing::info!(
                        ws_url = %cp.ws_url,
                        proxy_auth = config.proxy_base_user.is_some(),
                        default_country = ?config.proxy_default_country,
                        "chrome_proxy tier enabled"
                    );
                    js_renderers.push(Arc::new(renderer));
                }
            }
        }

        // Camoufox REST tier — a TOP-LEVEL block, NOT nested in the cdp guard
        // above (camoufox is REST, not CDP). The renderer is constructed
        // whenever an endpoint is configured, so an explicit per-request
        // `renderer = "camoufox"` pin can always reach it. Whether it
        // participates in the *auto* (non-pinned) chain is decided at request
        // time in `fetch_with_js` via `camoufox_in_auto` — a configured
        // endpoint with `include_in_auto = false` stays out of the auto ladder.
        #[cfg(feature = "camoufox")]
        {
            if let Some(cf) = config
                .camoufox
                .as_ref()
                .filter(|c| !c.base_url.trim().is_empty())
            {
                js_renderers.push(Arc::new(camoufox::CamoufoxRenderer::new(
                    "camoufox",
                    &cf.base_url,
                    &cf.api_key,
                    config.camoufox_timeout(),
                )) as Arc<dyn PageFetcher>);
                tracing::info!(
                    base_url = %cf.base_url,
                    include_in_auto = cf.include_in_auto,
                    "camoufox tier enabled"
                );
            } else if matches!(config.mode, RendererMode::Camoufox) {
                return Err(CrwError::ConfigError(
                    "renderer.mode = \"camoufox\" but [renderer.camoufox] base_url is not configured"
                        .into(),
                ));
            }
        }

        // Spawn the process-wide CDP telemetry sampler. Idempotent —
        // OnceLock guarantees a single task across all FallbackRenderer
        // instances. No-op on the `mode = none` early-return path above.
        #[cfg(feature = "cdp")]
        health_telemetry::spawn_once();

        if config.render_js_default == Some(true) && js_renderers.is_empty() {
            tracing::warn!(
                "render_js_default=true but no JS renderer is available; \
                 requests will fall back to HTTP via http_only_fallback"
            );
        }

        Ok(Self {
            http,
            js_renderers,
            render_js_default: config.render_js_default,
            latency_breakdown: config.latency_breakdown,
            auto_egress_escalation: config.auto_egress_escalation,
            chrome_hedge: config.chrome_hedge,
            hedge_sem: Arc::new(tokio::sync::Semaphore::new((config.pool_size / 2).max(1))),
            preferences: Arc::new(HostPreferences::with_defaults()),
            breakers: Arc::new(BreakerRegistry::with_defaults()),
            tier_timeouts: tier_timeouts_from(config),
            requests_per_second: 0.0,
            per_host_max_concurrent: 1,
            per_host_interactive_reserve: 1,
            antibot: config.antibot.clone(),
            proxy_rotator: None,
            http_ua: effective_ua.clone(),
            http_inject_stealth: inject_headers,
            http_timeout_ms,
            proxy_client_cache: std::sync::Mutex::new(std::collections::HashMap::new()),
            #[cfg(feature = "cdp")]
            chrome_pool,
            // Single source of truth for the opt-in policy: true only when
            // mode=camoufox (pinned) or mode=auto + include_in_auto. A
            // configured-but-not-opted-in endpoint stays out of the auto chain.
            #[cfg(feature = "camoufox")]
            camoufox_in_auto: config.camoufox_in_ladder(),
        })
    }

    /// Attach the config proxy rotator. Retained so scrape/crawl entry points
    /// can resolve a per-request proxy (via [`Self::pick_proxy_for_url`]) into
    /// the [`REQUEST_PROXY`] task-local; the HTTP and CDP paths then both consume
    /// that single resolved entry — no second pick. `None` is a no-op. Builder
    /// style so `new()`'s signature stays stable.
    pub fn with_proxy_rotator(
        mut self,
        rotator: Option<Arc<crw_core::ProxyRotator>>,
    ) -> CrwResult<Self> {
        self.proxy_rotator = rotator;
        Ok(self)
    }

    /// The HTTP fetcher to use for the current request. When `REQUEST_PROXY` is
    /// set (resolved once by the caller, honoring BYOP > config precedence),
    /// build a client bound to THAT exact proxy so the HTTP path egresses
    /// through the same proxy the CDP path uses. Hard-fails on a bad proxy
    /// (never a silent direct connection). When unset, use the shared
    /// (no-proxy or single-proxy) fetcher from `new()`.
    fn http_fetcher_for_request(&self) -> CrwResult<Arc<dyn PageFetcher>> {
        let Some(entry) = REQUEST_PROXY.try_with(|p| p.clone()).ok().flatten() else {
            return Ok(self.http.clone());
        };
        // Reuse a warm per-proxy client if we've built one before.
        if let Some(f) = self
            .proxy_client_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(entry.raw())
            .cloned()
        {
            return Ok(f);
        }
        let fetcher: Arc<dyn PageFetcher> = Arc::new(http_only::HttpFetcher::with_proxy(
            &self.http_ua,
            entry.raw(),
            self.http_inject_stealth,
            std::time::Duration::from_millis(self.http_timeout_ms),
        )?);
        let mut cache = self
            .proxy_client_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Bound growth under arbitrary BYOP proxies (config pools are small).
        if cache.len() >= 512 {
            cache.clear();
        }
        cache.insert(entry.raw().to_string(), fetcher.clone());
        Ok(fetcher)
    }

    /// Pick a proxy from the configured rotator for `host` (honoring the
    /// rotation strategy). `None` when no proxy is configured. Scrape/crawl
    /// entry points call this and scope the result into the [`REQUEST_PROXY`]
    /// task-local so the CDP/JS path egresses through the chosen proxy.
    pub fn pick_proxy(&self, host: Option<&str>) -> Option<Arc<crw_core::ProxyEntry>> {
        self.proxy_rotator
            .as_ref()
            .map(|r| Arc::new(r.pick(host).clone()))
    }

    /// True when a JS renderer (chrome / lightpanda / chrome_proxy) is wired in,
    /// so a `render_js` request can actually execute a page. The sitemap
    /// escalation arm uses this to skip pointless re-fetches of a challenged
    /// sitemap when no renderer could clear the wall anyway.
    pub fn js_capable(&self) -> bool {
        !self.js_renderers.is_empty() || self.auto_egress_escalation
    }

    /// Like [`Self::pick_proxy`] but derives the host key from a URL using the
    /// same normalization the HTTP fetcher and host limiter use — so the CDP
    /// `proxyServer` and the HTTP client land on the SAME sticky proxy.
    pub fn pick_proxy_for_url(&self, url: &str) -> Option<Arc<crw_core::ProxyEntry>> {
        self.proxy_rotator.as_ref()?;
        let host = url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(crate::preference::normalize_host));
        self.pick_proxy(host.as_deref())
    }

    /// Drain the chrome browser-context pool. Idempotent and a no-op when
    /// the pool is disabled. Call from the server's SIGTERM handler after
    /// the HTTP server has finished serving in-flight requests.
    #[cfg(feature = "cdp")]
    pub async fn shutdown_chrome_pool(&self, drain: std::time::Duration) {
        if let Some(pool) = self.chrome_pool.clone() {
            tracing::info!(
                drain_secs = drain.as_secs(),
                "draining chrome browser-context pool"
            );
            pool.shutdown(drain).await;
        }
    }

    /// No-op when the `cdp` feature is disabled — keeps caller code simple.
    #[cfg(not(feature = "cdp"))]
    pub async fn shutdown_chrome_pool(&self, _drain: std::time::Duration) {}

    /// Configure the process-wide per-host limiter (eTLD+1 keyed). Call once
    /// at startup with values from `CrawlerConfig`. Defaults: rps=0.0 (no
    /// interval floor), per-host cap=1 (strict politeness).
    pub fn with_host_limits(
        mut self,
        requests_per_second: f64,
        per_host_max_concurrent: u32,
        per_host_interactive_reserve: u32,
    ) -> Self {
        self.requests_per_second = requests_per_second;
        self.per_host_max_concurrent = per_host_max_concurrent;
        self.per_host_interactive_reserve = per_host_interactive_reserve;
        self
    }

    /// Access the host preferences cache (for admin endpoints, tests).
    pub fn preferences(&self) -> Arc<HostPreferences> {
        Arc::clone(&self.preferences)
    }

    /// Access the breaker registry (for tests).
    pub fn breakers(&self) -> Arc<BreakerRegistry> {
        Arc::clone(&self.breakers)
    }

    /// Names of the configured JS renderers in fallback order.
    /// Used for startup logs and tests — does not leak internal types.
    pub fn js_renderer_names(&self) -> Vec<&str> {
        self.js_renderers.iter().map(|r| r.name()).collect()
    }

    /// Whether this instance can actually capture a screenshot: at least one
    /// constructed JS renderer speaks CDP `Page.captureScreenshot`. Both the
    /// build features (no CDP feature ⇒ no tier is constructable) and the
    /// operator config (no `ws_url` ⇒ no tier) are reflected, because
    /// `js_renderers` is only populated when both hold.
    ///
    /// Shares [`renderer_can_screenshot`] with the request-time filter in
    /// [`Self::fetch_with_js`], so `/v1/capabilities` and the runtime can never
    /// disagree about which tiers can capture.
    pub fn supports_screenshot(&self) -> bool {
        self.js_renderer_names()
            .iter()
            .any(|name| renderer_can_screenshot(name))
    }

    /// Fetch a URL with smart mode: HTTP first, then JS if needed.
    ///
    /// When `render_js` is `None` (auto-detect), the renderer also escalates to
    /// JS rendering if the HTTP response looks like an anti-bot challenge page
    /// (Cloudflare "Just a moment...", etc.). The CDP renderer has built-in
    /// challenge retry logic that waits for non-interactive JS challenges to
    /// auto-resolve.
    pub async fn fetch(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        render_js: Option<bool>,
        wait_for_ms: Option<u64>,
        requested_renderer: Option<&str>,
        deadline: crw_core::Deadline,
    ) -> CrwResult<FetchResult> {
        // Phase 0 (latency-qn): time the whole fetch and emit a structured
        // breakdown event so bench runs can attribute p90 to a tier. The flag
        // is off by default, so the only cost on the hot path is one cheap
        // `Instant::now()` + a branch. The accepted tier is `rendered_with`,
        // which already distinguishes the HTTP fast-path from each JS renderer.
        if !self.latency_breakdown {
            return self
                .fetch_inner(
                    url,
                    headers,
                    render_js,
                    wait_for_ms,
                    requested_renderer,
                    deadline,
                )
                .await;
        }
        let t0 = std::time::Instant::now();
        let out = self
            .fetch_inner(
                url,
                headers,
                render_js,
                wait_for_ms,
                requested_renderer,
                deadline,
            )
            .await;
        let total_ms = t0.elapsed().as_millis() as u64;
        match &out {
            Ok(r) => tracing::info!(
                target: "latency_breakdown",
                url,
                total_ms,
                rendered_with = r.rendered_with.as_deref().unwrap_or("unknown"),
                content_len = r.html.len(),
                "scrape latency breakdown"
            ),
            Err(e) => tracing::info!(
                target: "latency_breakdown",
                url,
                total_ms,
                error = %e,
                "scrape latency breakdown (error)"
            ),
        }
        out
    }

    async fn fetch_inner(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        render_js: Option<bool>,
        wait_for_ms: Option<u64>,
        requested_renderer: Option<&str>,
        deadline: crw_core::Deadline,
    ) -> CrwResult<FetchResult> {
        // Per-eTLD+1 rate-limit + concurrency cap. Held across the entire
        // fetch (including any escalation to a JS renderer) so a host that
        // rate-limits HTTP doesn't get hammered by Chrome on retry.
        let host_key = url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(crate::preference::normalize_host));
        let _host_permit = if let Some(key) = host_key.as_deref() {
            let remaining = deadline.remaining();
            if remaining.is_zero() {
                return Err(CrwError::Timeout(
                    deadline.overrun().as_millis().max(1) as u64
                ));
            }
            match tokio::time::timeout(
                remaining,
                crate::host_limiter::acquire(
                    key,
                    self.requests_per_second,
                    self.per_host_max_concurrent as usize,
                    self.per_host_interactive_reserve as usize,
                ),
            )
            .await
            {
                Ok((permit, sleep)) => {
                    if !sleep.is_zero() {
                        let budget = deadline.remaining();
                        if sleep > budget {
                            return Err(CrwError::Timeout(sleep.as_millis().max(1) as u64));
                        }
                        tokio::time::sleep(sleep).await;
                    }
                    // Reserved per-host lane permit (interactive gets a dedicated
                    // slot, batch a bounded one). Held for the whole fetch by
                    // binding it to `_host_permit`.
                    Some(permit)
                }
                Err(_) => {
                    return Err(CrwError::Timeout(
                        deadline.overrun().as_millis().max(1) as u64
                    ));
                }
            }
        } else {
            None
        };

        let mut effective = resolve_render_js(render_js, self.render_js_default);
        // A screenshot is captured via CDP — it can only happen on the JS/CDP
        // path. Force `render_js = Some(true)` so the `Some(false)` / auto
        // (`None`) branches below don't return an HTTP-only result that never
        // reaches `fetch_with_js` (where the capture occurs). The HTTP-only,
        // camoufox and lightpanda renderers are also filtered out downstream.
        if effective != Some(true) && screenshot_requested() {
            effective = Some(true);
        }
        tracing::debug!(
            url,
            request_render_js = ?render_js,
            default_render_js = ?self.render_js_default,
            effective_render_js = ?effective,
            requested_renderer,
            "FallbackRenderer::fetch dispatching"
        );
        // A non-"auto" pinned renderer is a hard pin — failures must surface.
        let is_hard_pinned = matches!(requested_renderer, Some(name) if name != "auto");
        match effective {
            Some(false) => {
                let mut r = self
                    .http_fetcher_for_request()?
                    .fetch(url, headers, None, deadline)
                    .await?;
                stamp_http_decision(&mut r, requested_renderer);
                Ok(r)
            }
            Some(true) => {
                // Fetch via HTTP first to check content type — PDFs can't be JS-rendered.
                let mut http_result = self
                    .http_fetcher_for_request()?
                    .fetch(url, headers, None, deadline)
                    .await?;
                if http_result.content_type.as_deref() == Some("application/pdf") {
                    // A PDF has no rendered DOM to capture. A screenshot request
                    // on a PDF returns the parsed document with no `screenshot`
                    // field (ponytail: honest null — PDFs genuinely can't be
                    // screenshotted; not worth a warning the PDF parse path drops).
                    stamp_http_decision(&mut http_result, requested_renderer);
                    return Ok(http_result);
                }

                if self.js_renderers.is_empty() {
                    // A screenshot needs CDP — there is no HTTP fallback that can
                    // satisfy it. Fail closed rather than return a 200 with a null
                    // screenshot the caller explicitly asked for.
                    if screenshot_requested() {
                        return Err(CrwError::RendererError(
                            "a screenshot was requested but no JS renderer is available; \
                             configure a chrome/chrome_proxy tier"
                                .into(),
                        ));
                    }
                    tracing::warn!(
                        url,
                        "JS rendering requested but no renderer available — falling back to HTTP"
                    );
                    let mut result = http_result;
                    result.rendered_with = Some("http_only_fallback".to_string());
                    result.warning = Some("JS rendering was requested but no renderer is available. Content was fetched via HTTP only.".to_string());
                    result.warnings.push(
                        "JS rendering requested but no renderer available; HTTP fallback used"
                            .into(),
                    );
                    stamp_http_decision(&mut result, requested_renderer);
                    Ok(result)
                } else {
                    self.fetch_with_js(url, headers, wait_for_ms, requested_renderer, deadline)
                        .await
                }
            }
            None => {
                // In auto mode, an HTTP-layer failure (TargetUnreachable, body
                // decode mid-stream, oversize response, transient network) is
                // not terminal: if a JS renderer is available, escalate. Many
                // sites that reject reqwest's TLS/UA fingerprint succeed via a
                // real Chromium navigation. Bench analysis: 10/147 false
                // "unreachable" + 5/147 "http_502" map to this branch.
                let mut result = match self
                    .http_fetcher_for_request()?
                    .fetch(url, headers, None, deadline)
                    .await
                {
                    Ok(r) => r,
                    Err(e) if !self.js_renderers.is_empty() => {
                        tracing::info!(
                            url,
                            error = %e,
                            "HTTP fetch failed, escalating to JS renderer"
                        );
                        return self
                            .fetch_with_js(url, headers, wait_for_ms, requested_renderer, deadline)
                            .await
                            .map_err(|js_err| {
                                tracing::warn!("Both HTTP and JS failed: http={e}, js={js_err}");
                                // When the HTTP tier could not reach the origin AND the JS tier
                                // failed navigating to that same origin, the origin is the root
                                // cause: surface TargetUnreachable (422 — the caller handed us a
                                // dead target) instead of the JS tier's RendererError, which
                                // falls through to a 500 and reads as "our server broke".
                                //
                                // A JS failure can also be OUR fault (pool exhausted, CDP
                                // discovery failed, pinned renderer missing). Those keep their
                                // own error, or we would blame the caller for our outage.
                                match (&e, &js_err) {
                                    (CrwError::TargetUnreachable(_), js)
                                        if is_origin_navigation_failure(js) =>
                                    {
                                        e
                                    }
                                    _ => js_err,
                                }
                            });
                    }
                    Err(e) => return Err(e),
                };

                // PDFs don't need JS rendering — return immediately.
                if result.content_type.as_deref() == Some("application/pdf") {
                    stamp_http_decision(&mut result, requested_renderer);
                    return Ok(result);
                }

                let needs_js = detector::needs_js_rendering(&result.html);
                let cf_header_signal = result.warning.as_deref() == Some("cloudflare_mitigated");
                let is_generic_bot_wall = detector::looks_like_generic_bot_wall(&result.html);
                let is_blocked = cf_header_signal
                    || detector::looks_like_cloudflare_challenge(&result.html)
                    || is_generic_bot_wall;
                // Soft-block / soft-error status codes where the body often
                // contains real content despite the status header. Sources:
                //   - UA/header-based bot filters: 401, 403, 405, 406, 412
                //   - Rate limits: 429
                //   - Geo gates: 451
                //   - Origin overload: 503
                //   - "Not found" SPAs that 404 the route but render content
                //     via JS hydration: 404, 410
                //   - Origin error that still serves a usable page: 500
                // Firecrawl-comparison (April 2026 bench): the JS render
                // path recovered content in ~25/99 such cases that HTTP
                // alone could not.
                let is_auth_blocked = matches!(
                    result.status_code,
                    401 | 403 | 404 | 405 | 406 | 410 | 412 | 429 | 451 | 500 | 503
                );
                // Post-fetch thin-content trigger: HTTP returned 2xx but the
                // body has effectively no extractable text. Catches sites whose
                // SPA marker we don't recognize (no `id="root"`, no
                // `__next_data__`) yet still return a near-empty HTML shell.
                // Bench analysis showed 23/147 failures fall in this bucket
                // (seattletimes, espn, ionos, huduser, …).
                // Escalate a thin 2xx body ONLY when a browser would plausibly
                // reveal more (executable JS, or a meta-refresh redirect). A
                // script-less static doc (e.g. example.com) is already complete,
                // so a headless render just adds seconds for nothing. The
                // recognized-shell sites this bucket targets (seattletimes, espn,
                // …) all ship script bundles, so they still escalate.
                let is_2xx = (200..300).contains(&result.status_code);
                let is_thin_content = is_2xx
                    && detector::looks_like_thin_html(&result.html)
                    && detector::warrants_browser_retry(&result.html);

                if !self.js_renderers.is_empty()
                    && (needs_js || is_blocked || is_auth_blocked || is_thin_content)
                {
                    if is_auth_blocked {
                        tracing::info!(
                            url,
                            status_code = result.status_code,
                            "HTTP {} received, escalating to JS renderer",
                            result.status_code
                        );
                    } else if is_blocked {
                        tracing::info!(
                            url,
                            "Anti-bot challenge detected in HTTP response, escalating to JS renderer"
                        );
                        if is_generic_bot_wall {
                            tracing::info!(
                                url,
                                "Generic anti-bot interstitial detected, escalating to JS renderer"
                            );
                        }
                    } else if needs_js {
                        tracing::info!(url, "SPA shell detected, retrying with JS renderer");
                    } else {
                        tracing::info!(
                            url,
                            html_len = result.html.len(),
                            "HTTP 2xx but body is thin, escalating to JS renderer"
                        );
                    }
                    match self
                        .fetch_with_js(url, headers, wait_for_ms, requested_renderer, deadline)
                        .await
                    {
                        Ok(js_result) => Ok(js_result),
                        Err(e) if is_hard_pinned => {
                            // User explicitly pinned a renderer — surface the error
                            // instead of silently returning the (likely useless) HTTP body.
                            Err(e)
                        }
                        Err(e) => {
                            // For `is_auth_blocked` (4xx/5xx soft-block status codes), the
                            // HTTP body is almost certainly an error shell — falling back
                            // to it silently misleads the caller. Surface the JS failure
                            // through a warning so the post-extract layer can decide.
                            // For `needs_js` / `is_blocked` / `is_thin_content`, the HTTP
                            // body still has *some* useful content so the silent fallback
                            // remains the safer default.
                            if is_auth_blocked {
                                tracing::error!(
                                    url,
                                    status_code = result.status_code,
                                    "JS escalation failed for soft-block status; surfacing HTTP shell with warning: {e}"
                                );
                                let warning = format!("js_escalation_failed: {e}");
                                result.warning = Some(match result.warning.take() {
                                    Some(prev) => format!("{warning}; {prev}"),
                                    None => warning,
                                });
                            } else {
                                tracing::warn!(
                                    "JS rendering failed, falling back to HTTP result: {e}"
                                );
                            }
                            stamp_http_decision(&mut result, requested_renderer);
                            Ok(result)
                        }
                    }
                } else {
                    stamp_http_decision(&mut result, requested_renderer);
                    Ok(result)
                }
            }
        }
    }

    /// Minimum body text length for a JS-rendered result to be considered
    /// successful. If the rendered page has less visible text than this, the
    /// next renderer in the chain is tried.
    const MIN_RENDERED_TEXT_LEN: usize = 50;

    /// Pure classification of a JS-renderer result: the accept-gate + thin/block
    /// signals, NO side-effects. Shared by the serial escalation loop and the
    /// conditional hedge so both apply the identical accept criteria (the red
    /// line: hedge must be provably ≡ serial on success/recall).
    fn classify_js_attempt(&self, result: &FetchResult) -> JsAttemptClass {
        let text_len = html_body_text_len(&result.html);
        let is_placeholder = detector::looks_like_loading_placeholder(&result.html);
        let failed_render = detector::looks_like_failed_render(&result.html);
        let is_bot_wall = detector::looks_like_generic_bot_wall(&result.html);
        let vendor_block = detector::looks_like_vendor_block(&result.html);
        let is_status_blocked = matches!(
            result.status_code,
            401 | 403 | 404 | 405 | 406 | 410 | 412 | 429 | 451 | 500 | 503
        );
        let antibot = if self.antibot.enabled {
            crw_extract::antibot::classify(Some(result.status_code), &result.html)
        } else {
            crw_extract::antibot::AntibotResult::none()
        };
        let antibot_blocked = self.antibot.escalate_in_failover && antibot.signal.is_blocked();
        // Egress-recoverable hard-block subset (drives the gated chrome_proxy arm).
        let hard_block = matches!(result.status_code, 401 | 403 | 429 | 503)
            || (520..=530).contains(&result.status_code)
            || is_bot_wall
            || vendor_block.is_some()
            || antibot.signal.is_blocked();
        let acceptable = text_len >= Self::MIN_RENDERED_TEXT_LEN
            && !is_placeholder
            && failed_render.is_none()
            && !is_bot_wall
            && vendor_block.is_none()
            && !is_status_blocked
            && !antibot_blocked;
        JsAttemptClass {
            text_len,
            is_placeholder,
            failed_render,
            is_bot_wall,
            vendor_block,
            is_status_blocked,
            antibot,
            antibot_blocked,
            hard_block,
            acceptable,
        }
    }

    /// Conditional hedge: race lightpanda + chrome CONCURRENTLY (chrome's render
    /// clock starts immediately instead of after lightpanda fails) and take the
    /// best result by tier priority. Returns `None` if a breaker was open (caller
    /// falls back to serial). Success/recall ≡ serial:
    ///   * Rule A: among gate-passing results, lightpanda wins (serial accepts
    ///     lightpanda when it passes, never seeing chrome); a faster-arriving
    ///     chrome only wins if lightpanda is NOT acceptable.
    ///   * Rule B: if neither passes, return the richest-HTML thin (== serial's
    ///     thin stitch).
    ///   * Rule C: record breaker/preference side-effects only for tiers that
    ///     actually COMPLETED (the cancelled loser — dropped on the other's
    ///     accept — records nothing); its in-flight render is reaped by the
    ///     PoolGuard Drop reaper.
    #[allow(clippy::too_many_arguments)] // url/headers/wait/deadline/host mirror fetch_with_js
    async fn try_hedge(
        &self,
        lp: &Arc<dyn PageFetcher>,
        chrome: &Arc<dyn PageFetcher>,
        url: &str,
        headers: &HashMap<String, String>,
        wait_for_ms: Option<u64>,
        deadline: crw_core::Deadline,
        host: &str,
    ) -> CrwResult<Option<HedgeOutcome>> {
        // Breaker gates (mirror serial). If either tier's breaker is open, bail to
        // serial so its skip/leak-through handling applies.
        let (lp_permit, lp_guard) = self
            .breakers
            .acquire_with_guard(host, RendererKind::Lightpanda)
            .await;
        if lp_permit == Permit::Rejected {
            drop(lp_guard);
            return Ok(None);
        }
        let (ch_permit, ch_guard) = self
            .breakers
            .acquire_with_guard(host, RendererKind::Chrome)
            .await;
        if ch_permit == Permit::Rejected {
            drop(lp_guard);
            drop(ch_guard);
            return Ok(None);
        }
        // Both `acquire_with_guard` calls above await, so the budget may have drained
        // since the caller's floor check. Re-check before dispatching, or the floor is
        // only advisory here. Bail to the serial loop, which applies its own floor and
        // records the skip.
        if deadline.remaining() < MIN_TIER_BUDGET {
            drop(lp_guard);
            drop(ch_guard);
            return Ok(None);
        }

        let mut lp_guard = Some(lp_guard);
        let mut ch_guard = Some(ch_guard);

        // Race both on the CURRENT task (select!, not spawn) so REQUEST_PROXY /
        // REQUEST_COUNTRY task-locals propagate into each fetch.
        let lp_fut = lp.fetch(url, headers, wait_for_ms, deadline);
        let chrome_fut = chrome.fetch(url, headers, wait_for_ms, deadline);
        tokio::pin!(lp_fut, chrome_fut);
        let (mut lp_done, mut ch_done) = (false, false);
        let mut lp_res: Option<CrwResult<FetchResult>> = None;
        let mut ch_res: Option<CrwResult<FetchResult>> = None;
        while !(lp_done && ch_done) {
            tokio::select! {
                biased;
                r = &mut lp_fut, if !lp_done => {
                    lp_done = true;
                    let accept = matches!(&r, Ok(res) if self.classify_js_attempt(res).acceptable);
                    lp_res = Some(r);
                    // Rule A: lightpanda authoritative — accept now, drop chrome.
                    if accept {
                        break;
                    }
                }
                r = &mut chrome_fut, if !ch_done => {
                    ch_done = true;
                    let ch_accept = matches!(&r, Ok(res) if self.classify_js_attempt(res).acceptable);
                    ch_res = Some(r);
                    // chrome may finish first; only accept it early once lightpanda
                    // is known NOT acceptable (else wait for lightpanda — Rule A).
                    if lp_done {
                        let lp_accept = matches!(&lp_res, Some(Ok(res)) if self.classify_js_attempt(res).acceptable);
                        if !lp_accept && ch_accept {
                            break;
                        }
                    }
                }
            }
        }
        // The still-pending future (if any) drops at scope end → PoolGuard reaper.

        // Finalize. Record side-effects only for COMPLETED tiers (Some result).
        let lp_accept =
            matches!(&lp_res, Some(Ok(res)) if self.classify_js_attempt(res).acceptable);
        let ch_accept =
            matches!(&ch_res, Some(Ok(res)) if self.classify_js_attempt(res).acceptable);

        // Rule A: lightpanda wins if acceptable.
        if lp_accept {
            let mut r = lp_res.unwrap().unwrap();
            self.record_hedge_success(host, RendererKind::Lightpanda, &r, &mut lp_guard)
                .await;
            // chrome cancelled or thin → no record (Rule C).
            r.credit_cost = credit_for(RendererKind::Lightpanda);
            r.render_decision = Some(RenderDecision::AutoDefault {
                chosen: RendererKind::Lightpanda,
            });
            return Ok(Some(HedgeOutcome::Accepted(r)));
        }
        // lightpanda completed thin → record it (serial would have).
        let mut saw_hard_block = false;
        if let Some(Ok(res)) = &lp_res {
            let cls = self.classify_js_attempt(res);
            saw_hard_block |= cls.hard_block;
            self.record_hedge_thin(host, RendererKind::Lightpanda, &cls, &mut lp_guard)
                .await;
        }
        if ch_accept {
            let mut r = ch_res.unwrap().unwrap();
            self.record_hedge_success(host, RendererKind::Chrome, &r, &mut ch_guard)
                .await;
            r.credit_cost = credit_for(RendererKind::Chrome);
            r.render_decision = Some(RenderDecision::Failover {
                chain: vec![RendererKind::Lightpanda, RendererKind::Chrome],
                reason: FailoverErrorKind::Other,
            });
            return Ok(Some(HedgeOutcome::Accepted(r)));
        }
        // chrome completed thin → record it.
        if let Some(Ok(res)) = &ch_res {
            let cls = self.classify_js_attempt(res);
            saw_hard_block |= cls.hard_block;
            self.record_hedge_thin(host, RendererKind::Chrome, &cls, &mut ch_guard)
                .await;
        }

        // Rule B: best-thin = richest HTML among completed Ok results.
        let thin = [lp_res, ch_res]
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .max_by_key(|r| r.html.len());
        match thin {
            Some(r) => Ok(Some(HedgeOutcome::Thin(r, saw_hard_block))),
            // Both tiers errored — let the caller fall back to serial for its
            // richer error handling rather than inventing an error here.
            None => Ok(None),
        }
    }

    /// Record a hedge winner's success side-effects (breaker + preference + guard).
    async fn record_hedge_success(
        &self,
        host: &str,
        k: RendererKind,
        result: &FetchResult,
        guard: &mut Option<ProbeGuard>,
    ) {
        if !host.is_empty() {
            let outcome = if result.truncated {
                BreakerOutcome::Truncated
            } else {
                BreakerOutcome::Success
            };
            self.breakers.record_outcome(host, k, outcome).await;
            self.preferences.record_success(host).await;
        }
        if let Some(g) = guard.take() {
            g.disarm();
        }
    }

    /// Record a hedge thin/blocked tier's failure side-effects.
    async fn record_hedge_thin(
        &self,
        host: &str,
        k: RendererKind,
        cls: &JsAttemptClass,
        guard: &mut Option<ProbeGuard>,
    ) {
        if !host.is_empty() {
            self.breakers
                .record_outcome(host, k, BreakerOutcome::RenderError)
                .await;
            if k == RendererKind::Lightpanda {
                let err_kind = if cls.is_status_blocked || cls.is_bot_wall || cls.antibot_blocked {
                    FailoverErrorKind::AntibotBlock
                } else {
                    FailoverErrorKind::PlaceholderContent
                };
                let _ = self.preferences.record_failure(host, &err_kind).await;
            }
        }
        // Thin attempt → leave the probe guard armed (drops as a no-op).
        let _ = guard;
    }

    async fn fetch_with_js(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        wait_for_ms: Option<u64>,
        requested_renderer: Option<&str>,
        deadline: crw_core::Deadline,
    ) -> CrwResult<FetchResult> {
        let host = host_of(url);
        let is_user_pinned = matches!(requested_renderer, Some(name) if name != "auto");
        if let Some(pinned) = requested_renderer
            && let Some(kind) = renderer_kind_for(pinned)
        {
            metrics()
                .user_pin_total
                .with_label_values(&[kind.as_str()])
                .inc();
        }

        // Filter the JS pool down to a hard-pinned renderer when one was named.
        // "auto" or `None` means "use the configured chain".
        //
        // A pinned request (`Some(name)` where name != "auto") is matched by
        // exact name and BYPASSES the camoufox auto-exclusion — an explicit
        // `renderer = "camoufox"` pin always reaches the (constructed) tier even
        // when `include_in_auto = false`. The exclusion applies ONLY to the
        // non-pinned auto chain.
        let mut renderers: Vec<&Arc<dyn PageFetcher>> = match requested_renderer {
            Some(name) if name != "auto" => self
                .js_renderers
                .iter()
                .filter(|r| r.name() == name)
                .collect(),
            _ => {
                #[cfg(feature = "camoufox")]
                {
                    let in_auto = self.camoufox_in_auto;
                    self.js_renderers
                        .iter()
                        .filter(|r| in_auto || r.name() != "camoufox")
                        .collect()
                }
                #[cfg(not(feature = "camoufox"))]
                {
                    self.js_renderers.iter().collect()
                }
            }
        };

        // LightPanda has no upstream-proxy support: when a proxy is active for
        // this request, drop it so the rotated/sticky egress IP is honored
        // (vanilla Chrome applies it via a per-context `proxyServer`). Fail
        // CLOSED — if filtering leaves no proxy-capable JS renderer, return a
        // hard error rather than silently navigating direct through LightPanda
        // and leaking the host's real IP.
        let proxy_active = REQUEST_PROXY.try_with(|p| p.is_some()).unwrap_or(false);
        if proxy_active {
            renderers.retain(|r| r.name() != "lightpanda");
            if renderers.is_empty() {
                return Err(CrwError::RendererError(
                    "a proxy is required for this request but the only available JS \
                     renderer (lightpanda) cannot route through a proxy; configure a \
                     chrome/chrome_proxy tier to use proxies with JS rendering"
                        .into(),
                ));
            }
        }

        // Drop the tiers that cannot capture (see `renderer_can_screenshot` —
        // the same predicate `/v1/capabilities` reports) and fail CLOSED if that
        // empties the chain, rather than returning a screenshot-less result the
        // caller asked for (mirrors the proxy retain above). Applies even to a
        // hard pin: pinning camoufox/lightpanda + requesting a screenshot is
        // unsatisfiable.
        if screenshot_requested() {
            renderers.retain(|r| renderer_can_screenshot(r.name()));
            if renderers.is_empty() {
                return Err(CrwError::RendererError(
                    "a screenshot was requested but no CDP-capable Chrome renderer is \
                     available; lightpanda and camoufox cannot capture screenshots — \
                     configure a chrome/chrome_proxy tier"
                        .into(),
                ));
            }
        }
        // Phase 2 (latency-qn): gated auto-egress. Pull chrome_proxy OUT of the
        // normal ladder and hold it as a hard-block-only recovery arm fired ONCE
        // after the ladder (below), with a reserved deadline budget. A naive
        // always-on chrome_proxy ladder tier is net-negative (bench: success
        // −2pp, p90 +69%) because the slow residential tier burns the deadline
        // on every escalation; gating it to genuine hard-blocks keeps the
        // recovery without the regression. Only in auto mode and when the
        // request isn't already proxied (that path wants chrome_proxy in-ladder).
        let auto_egress_arm: Option<Arc<dyn PageFetcher>> =
            if self.auto_egress_escalation && !is_user_pinned && !proxy_active {
                let arm = self
                    .js_renderers
                    .iter()
                    .find(|r| r.name() == "chrome_proxy")
                    .cloned();
                renderers.retain(|r| r.name() != "chrome_proxy");
                arm
            } else {
                None
            };

        // Auto mode: if this host has been promoted, try Chrome first.
        if !is_user_pinned
            && let Some(RendererKind::Chrome) = self.preferences.preferred(&host).await
        {
            // 3-tier rank: chrome first, then the residential chrome_proxy,
            // then everything lighter. A stable binary key would yield
            // `[chrome, lightpanda, chrome_proxy]` — escalating a chrome
            // block to lightpanda (same WAF, lighter fingerprint) before
            // ever reaching the residential tier.
            renderers.sort_by_key(|r| match r.name() {
                "chrome" => 0,
                "chrome_proxy" => 1,
                _ => 2,
            });
            tracing::debug!(host = %host, "host promoted to chrome by preference learner");
        }

        if renderers.is_empty() {
            let available = self.js_renderer_names();
            return Err(CrwError::RendererError(format!(
                "requested renderer '{}' not in pool [{}]",
                requested_renderer.unwrap_or("auto"),
                available.join(", ")
            )));
        }

        // Track the chain we attempted so we can populate
        // `RenderDecision::Failover` when nothing succeeded outright.
        let mut chain: Vec<RendererKind> = Vec::new();
        let mut breaker_skipped: Vec<RendererKind> = Vec::new();
        let mut last_error = None;
        let mut last_failover_reason: Option<FailoverErrorKind> = None;
        let mut thin_result: Option<FetchResult> = None;
        // Phase 2: did any ladder attempt end in a hard block (egress-recoverable
        // subset: 401/403/429/503/520-530 or a bot-wall/vendor/antibot wall)?
        // Drives the gated chrome_proxy recovery arm below. Excludes
        // 404/410/412/451/500 (a different egress IP won't fix those).
        let mut saw_hard_block = false;
        // Snapshot for the leak-through fallback below. The main loop
        // consumes `renderers`; we keep a parallel reference list so a
        // single skipped renderer can still get a shot when its host
        // breaker is closed.
        let renderers_snapshot: Vec<&Arc<dyn PageFetcher>> = renderers.clone();

        // latency-qn conditional hedge: when lightpanda is first (cheap-first, not
        // promoted to chrome) and chrome is present, race them concurrently so
        // chrome's render clock starts immediately instead of after lightpanda
        // fails. Headroom-gated (try_acquire) so it can't deadlock the pool; on no
        // permit / open breaker / both-errored it falls through to the serial loop.
        let mut hedge_done = false;
        if self.chrome_hedge
            && !is_user_pinned
            && !proxy_active
            // Same degenerate-budget guard as the serial loop: the hedge dispatches
            // both CDP tiers directly, bypassing that check. Prod runs with
            // CRW_CHROME_HEDGE=true, so this is load-bearing, not defensive.
            && deadline.remaining() >= MIN_TIER_BUDGET
            && renderers.first().map(|r| r.name()) == Some("lightpanda")
            && renderers.iter().any(|r| r.name() == "chrome")
            && let Ok(_permit) = self.hedge_sem.clone().try_acquire_owned()
        {
            let lp = renderers
                .iter()
                .find(|r| r.name() == "lightpanda")
                .expect("checked above");
            let chrome = renderers
                .iter()
                .find(|r| r.name() == "chrome")
                .expect("checked above");
            match self
                .try_hedge(lp, chrome, url, headers, wait_for_ms, deadline, &host)
                .await
            {
                Ok(Some(HedgeOutcome::Accepted(r))) => return Ok(r),
                Ok(Some(HedgeOutcome::Thin(r, hb))) => {
                    thin_result = Some(r);
                    saw_hard_block |= hb;
                    chain.push(RendererKind::Lightpanda);
                    chain.push(RendererKind::Chrome);
                    hedge_done = true;
                }
                // breaker open / both-errored → fall back to the serial loop.
                Ok(None) => {}
                Err(e) => last_error = Some(e),
            }
        }

        for renderer in renderers {
            if hedge_done {
                break;
            }
            let kind = renderer_kind_for(renderer.name());

            // Skip empty hosts: don't pollute breaker/preference caches
            // with the "" key when URL parsing failed.
            let trackable = kind.filter(|_| !host.is_empty());

            // A tier-side skip on a *partial* budget stays removed (86dd10f): letting
            // chrome attempt with a partial-DOM budget beats aborting pre-flight on
            // legitimately-slow tail URLs, and classify_outcome ignores DeadlineClamped
            // so the breaker isn't poisoned. What is reinstated here is narrower: a
            // *degenerate* budget. A CDP attempt cannot even finish its handshake in
            // single-digit milliseconds, so it returns a fabricated `Timeout after 5ms`
            // that pollutes logs and burns a pool slot. Measured in prod: 432 of 536
            // escalations ran with <50ms of budget. Skip those, and only those.
            //
            // Note this is skip-*without*-attempting, distinct from the post-hoc
            // DeadlineClamped classification, which still only applies to tiers that
            // were actually invoked.
            let remaining = deadline.remaining();
            if remaining < MIN_TIER_BUDGET {
                tracing::debug!(
                    renderer = renderer.name(),
                    remaining_ms = remaining.as_millis() as u64,
                    "budget below minimum tier budget, skipping renderer"
                );
                if let Some(k) = kind {
                    // Deliberately NOT `breaker_skipped`: that vec means "the circuit
                    // breaker rejected this tier" and gates the leak-through arm.
                    metrics()
                        .render_route_decision_total
                        .with_label_values(&[k.as_str(), "budgetSkipped"])
                        .inc();
                }
                // Preserve the status code a starved request returns today. The tier we
                // are skipping would have been invoked with `remaining`, timed out, and
                // written `CrwError::Timeout` here — overwriting any earlier error, as
                // every other `last_error` assignment in this function does. Assign
                // unconditionally for the same reason: `get_or_insert_with` would let an
                // earlier tier's `RendererError` survive and map to 500 instead of 504.
                //
                // Report the budget the tier would have had, matching what the CDP tier
                // reports when invoked and clamped (`Timeout after 5ms`). `overrun()` is
                // 0 whenever the deadline has not actually expired — the common case
                // here (1-499ms left).
                last_error = Some(CrwError::Timeout(remaining.as_millis().max(1) as u64));
                continue;
            }

            // Consult breaker for tracked renderers. Untracked names (e.g.
            // "playwright") bypass the breaker for now.
            let mut probe_guard: Option<ProbeGuard> = None;
            if let Some(k) = trackable {
                let (permit, guard) = self.breakers.acquire_with_guard(&host, k).await;
                if permit == Permit::Rejected {
                    tracing::info!(
                        renderer = renderer.name(),
                        host = %host,
                        "circuit breaker open, skipping renderer"
                    );
                    metrics()
                        .render_route_decision_total
                        .with_label_values(&[k.as_str(), "breakerSkipped"])
                        .inc();
                    breaker_skipped.push(k);
                    drop(guard); // not Probe — drop is a no-op
                    continue;
                }
                probe_guard = Some(guard);
            }

            // `acquire_with_guard` awaits, so the budget may have drained while we
            // waited for a breaker permit. Re-check before dispatching, or the floor
            // above is only advisory. Dropping `probe_guard` here cancels the probe
            // (see `ProbeGuard::drop`), so the breaker is left as we found it.
            let remaining = deadline.remaining();
            if remaining < MIN_TIER_BUDGET {
                tracing::debug!(
                    renderer = renderer.name(),
                    remaining_ms = remaining.as_millis() as u64,
                    "budget drained while acquiring breaker permit, skipping renderer"
                );
                if let Some(k) = kind {
                    metrics()
                        .render_route_decision_total
                        .with_label_values(&[k.as_str(), "budgetSkipped"])
                        .inc();
                }
                last_error = Some(CrwError::Timeout(remaining.as_millis().max(1) as u64));
                continue;
            }

            if let Some(k) = kind {
                chain.push(k);
            }

            // Capture pre-call context so post-await classification is
            // race-free against deadline drift.
            let attempt_ctx = {
                let remaining = deadline.remaining();
                let tier_budget = kind
                    .and_then(|k| self.tier_timeouts.get(&k).copied())
                    .unwrap_or(remaining);
                AttemptContext::capture(remaining, tier_budget)
            };
            // Phase 1 (latency-qn): per-attempt timing. The whole-fetch wrapper
            // only records total + accepted tier; this records each tier's
            // wall time + outcome so a bench run can tell whether the p90 tail
            // is stacked failed-tier time (a hedge would cut it) or the final
            // accepted render itself (a hedge would NOT). Feeds the Phase 1.5
            // kill-gate. Off in prod (gated by `latency_breakdown`).
            let attempt_start = std::time::Instant::now();
            let attempt_outcome = renderer.fetch(url, headers, wait_for_ms, deadline).await;
            if self.latency_breakdown {
                let attempt_ms = attempt_start.elapsed().as_millis() as u64;
                let tier = renderer.name();
                match &attempt_outcome {
                    Ok(r) => tracing::info!(
                        target: "latency_breakdown",
                        url, tier, attempt_ms,
                        status = r.status_code,
                        html_len = r.html.len(),
                        "hedge attempt"
                    ),
                    Err(e) => tracing::info!(
                        target: "latency_breakdown",
                        url, tier, attempt_ms,
                        error = %e,
                        "hedge attempt (error)"
                    ),
                }
            }
            match attempt_outcome {
                Ok(mut result) => {
                    let text_len = html_body_text_len(&result.html);
                    let is_placeholder = detector::looks_like_loading_placeholder(&result.html);
                    let failed_render = detector::looks_like_failed_render(&result.html);
                    let is_bot_wall = detector::looks_like_generic_bot_wall(&result.html);
                    let vendor_block = detector::looks_like_vendor_block(&result.html);
                    // Mirrors the HTTP-tier escalation set (lib.rs:658). A JS
                    // renderer can return 200 with bot HTML or 403 with content
                    // — without this check, both slip through as "valid".
                    let is_status_blocked = matches!(
                        result.status_code,
                        401 | 403 | 404 | 405 | 406 | 410 | 412 | 429 | 451 | 500 | 503
                    );
                    // The comprehensive 3-tier antibot classifier. The
                    // `detector` heuristics above only know a fixed phrase
                    // list + 8 named vendors; `classify()` additionally
                    // recognises Reddit-class WAF pages ("blocked by network
                    // security") served with HTTP 200 that otherwise slip
                    // through as success. Always runs for telemetry when
                    // `enabled`; only forces escalation when
                    // `escalate_in_failover` is on (the kill switch).
                    let antibot = if self.antibot.enabled {
                        crw_extract::antibot::classify(Some(result.status_code), &result.html)
                    } else {
                        crw_extract::antibot::AntibotResult::none()
                    };
                    let antibot_blocked =
                        self.antibot.escalate_in_failover && antibot.signal.is_blocked();
                    // Phase 2: track hard-block (egress-recoverable) outcomes for
                    // the gated chrome_proxy arm. Hard-block status subset only
                    // (not 404/410/412/451/500) + interstitial walls.
                    if matches!(result.status_code, 401 | 403 | 429 | 503)
                        || (520..=530).contains(&result.status_code)
                        || is_bot_wall
                        || vendor_block.is_some()
                        || antibot.signal.is_blocked()
                    {
                        saw_hard_block = true;
                    }
                    if text_len >= Self::MIN_RENDERED_TEXT_LEN
                        && !is_placeholder
                        && failed_render.is_none()
                        && !is_bot_wall
                        && vendor_block.is_none()
                        && !is_status_blocked
                        && !antibot_blocked
                    {
                        // Capture the promotion state BEFORE record_success
                        // clears the latch — otherwise AutoPromoted decisions
                        // race against the success path and downgrade to AutoDefault.
                        let was_promoted = matches!(
                            self.preferences.preferred(&host).await,
                            Some(RendererKind::Chrome)
                        );
                        if let Some(k) = trackable {
                            // Treat truncated-but-valid as Truncated (ignored
                            // by default per BreakerConfig.count_truncated_as_failure).
                            let outcome = if result.truncated {
                                BreakerOutcome::Truncated
                            } else {
                                BreakerOutcome::Success
                            };
                            self.breakers.record_outcome(&host, k, outcome).await;
                            self.preferences.record_success(&host).await;
                            metrics()
                                .render_route_decision_total
                                .with_label_values(&[k.as_str(), "success"])
                                .inc();
                            metrics()
                                .host_preferences_size
                                .set(self.preferences.size() as i64);
                        }
                        if let Some(g) = probe_guard.take() {
                            g.disarm();
                        }
                        // Populate routing metadata + per-renderer credit.
                        if let Some(k) = kind {
                            result.credit_cost = credit_for(k);
                            result.render_decision = Some(if is_user_pinned {
                                RenderDecision::UserPinned { renderer: k }
                            } else if !breaker_skipped.is_empty() {
                                RenderDecision::BreakerSkipped {
                                    skipped: breaker_skipped[0],
                                    chosen: k,
                                }
                            } else if chain.len() > 1 {
                                RenderDecision::Failover {
                                    chain: chain.clone(),
                                    reason: last_failover_reason
                                        .clone()
                                        .unwrap_or(FailoverErrorKind::Other),
                                }
                            } else if was_promoted && k == RendererKind::Chrome {
                                RenderDecision::AutoPromoted {
                                    chosen: k,
                                    from: RendererKind::Lightpanda,
                                    reason: "host preference learner".into(),
                                }
                            } else {
                                RenderDecision::AutoDefault { chosen: k }
                            });
                        }
                        return Ok(result);
                    }
                    // Treat thin/placeholder/failed as a soft failure for
                    // breaker + preference purposes.
                    let err_kind = match failed_render {
                        Some(detector::FailedRenderReason::NextJsClientError) => {
                            FailoverErrorKind::NextJsClientError
                        }
                        Some(detector::FailedRenderReason::ReactMinifiedError) => {
                            FailoverErrorKind::NextJsClientError
                        }
                        Some(detector::FailedRenderReason::EmptyNextRoot) => {
                            FailoverErrorKind::EmptyNextRoot
                        }
                        None if vendor_block.is_some() => FailoverErrorKind::VendorBlock,
                        None if is_status_blocked => FailoverErrorKind::StatusBlocked,
                        None if is_placeholder => FailoverErrorKind::PlaceholderContent,
                        None if is_bot_wall => FailoverErrorKind::PlaceholderContent,
                        // The classifier caught a block the detector missed.
                        None if antibot_blocked => FailoverErrorKind::AntibotBlock,
                        None => FailoverErrorKind::PlaceholderContent,
                    };
                    last_failover_reason = Some(err_kind.clone());
                    if let Some(k) = trackable {
                        // Thin/placeholder/failed render → classify against
                        // attempt context so deadline-clamped attempts don't
                        // poison the breaker.
                        let outcome = classify_outcome(false, false, false, &attempt_ctx);
                        self.breakers.record_outcome(&host, k, outcome).await;
                        if k == RendererKind::Lightpanda
                            && let Some(target) =
                                self.preferences.record_failure(&host, &err_kind).await
                        {
                            metrics()
                                .host_preferences_promotions_total
                                .with_label_values(&[k.as_str(), target.as_str()])
                                .inc();
                            tracing::info!(
                                host = %host,
                                "host promoted by preference learner: {} -> {}",
                                k.as_str(),
                                target.as_str()
                            );
                        }
                    }
                    if let Some(g) = probe_guard.take() {
                        g.disarm();
                    }
                    if let Some(vendor) = vendor_block {
                        metrics()
                            .vendor_block_total
                            .with_label_values(&[vendor])
                            .inc();
                        tracing::warn!(
                            renderer = renderer.name(),
                            url,
                            vendor,
                            "vendor anti-bot block detected"
                        );
                    }
                    // Emit the antibot signal regardless of `escalate_in_failover`
                    // — a pre-flip dashboard of escalation pressure.
                    if antibot.signal.is_blocked() {
                        metrics()
                            .antibot_escalation_total
                            .with_label_values(&[antibot.signal.class_name()])
                            .inc();
                        tracing::warn!(
                            renderer = renderer.name(),
                            url,
                            signal = antibot.signal.class_name(),
                            reason = %antibot.reason,
                            status_code = result.status_code,
                            text_len,
                            escalated = antibot_blocked,
                            "antibot classifier flagged a block"
                        );
                    }
                    tracing::info!(
                        renderer = renderer.name(),
                        text_len,
                        is_placeholder,
                        is_bot_wall,
                        vendor_block,
                        is_status_blocked,
                        antibot_signal = antibot.signal.class_name(),
                        antibot_blocked,
                        status_code = result.status_code,
                        failed_render = ?failed_render,
                        "JS renderer returned thin/placeholder/failed content, trying next renderer"
                    );
                    // Annotate the result so it can surface through `thin_result`
                    // if no later renderer succeeds. Preserves any warning the
                    // renderer set, but adds the failover reason. We keep the
                    // first thin result as the body to return (no point in
                    // accumulating bodies), but stitch later renderers'
                    // warnings onto it so debug output reflects every attempt.
                    let mut annotated = result;
                    let attempt_warning = if let Some(reason) = failed_render {
                        format!(
                            "{} returned a failed render ({})",
                            renderer.name(),
                            reason.as_str()
                        )
                    } else if is_placeholder {
                        format!("{} returned a loading placeholder", renderer.name())
                    } else if let Some(vendor) = vendor_block {
                        format!(
                            "{} returned a vendor anti-bot block ({vendor})",
                            renderer.name()
                        )
                    } else if is_bot_wall {
                        format!(
                            "{} returned a generic anti-bot interstitial",
                            renderer.name()
                        )
                    } else if is_status_blocked {
                        format!(
                            "{} returned HTTP {} (treated as blocked)",
                            renderer.name(),
                            annotated.status_code
                        )
                    } else if antibot_blocked {
                        format!(
                            "{} returned an anti-bot block ({}: {})",
                            renderer.name(),
                            antibot.signal.class_name(),
                            antibot.reason
                        )
                    } else {
                        format!(
                            "{} returned thin content (text_len={text_len})",
                            renderer.name()
                        )
                    };
                    if is_bot_wall || vendor_block.is_some() || is_status_blocked || antibot_blocked
                    {
                        // Surface bot-wall as a RendererError so, if every
                        // renderer in the chain hits a wall, the final error
                        // (line ~1052) carries an actionable message.
                        // RendererError maps to FailoverErrorKind::LightpandaCrash
                        // via classify_renderer_error — that's intentional:
                        // bot-wall hosts SHOULD be promoted to Chrome by the
                        // host preference learner, since LightPanda lacks the
                        // TLS/header fingerprint to clear them.
                        let msg = if let Some(v) = vendor_block {
                            format!("{} returned a vendor anti-bot block ({v})", renderer.name())
                        } else if is_status_blocked {
                            format!(
                                "{} returned HTTP {} (treated as blocked)",
                                renderer.name(),
                                annotated.status_code
                            )
                        } else if is_bot_wall {
                            format!(
                                "{} returned a generic anti-bot interstitial",
                                renderer.name()
                            )
                        } else {
                            format!(
                                "{} returned an anti-bot block ({}: {})",
                                renderer.name(),
                                antibot.signal.class_name(),
                                antibot.reason
                            )
                        };
                        last_error = Some(CrwError::RendererError(msg));
                    }
                    annotated.warnings.push(attempt_warning.clone());
                    annotated.warning = Some(match annotated.warning {
                        Some(prev) => format!("{prev}; {attempt_warning}"),
                        None => attempt_warning.clone(),
                    });
                    thin_result = Some(match thin_result {
                        None => annotated,
                        Some(existing) => {
                            // Prefer the larger HTML when stitching thin
                            // results — a later renderer (e.g. chrome) often
                            // returns a CAPTCHA shell that, while small,
                            // contains anti-bot markers absent from an even
                            // smaller earlier shell. Diagnostics & block
                            // detection then have something to match on.
                            let (mut keeper, dropped) =
                                if annotated.html.len() > existing.html.len() {
                                    (annotated, existing)
                                } else {
                                    (existing, annotated)
                                };
                            keeper.warnings.push(attempt_warning.clone());
                            keeper.warning = Some(match keeper.warning {
                                Some(prev) => format!("{prev}; {attempt_warning}"),
                                None => attempt_warning,
                            });
                            // Carry over any extra warnings from the dropped
                            // attempt so debug output stays complete.
                            for w in dropped.warnings {
                                if !keeper.warnings.contains(&w) {
                                    keeper.warnings.push(w);
                                }
                            }
                            keeper
                        }
                    });
                }
                Err(e) => {
                    tracing::warn!(renderer = renderer.name(), "JS renderer failed: {e}");
                    let err_kind = classify_renderer_error(&e);
                    last_failover_reason = Some(err_kind.clone());
                    if let Some(k) = trackable {
                        let was_timeout = matches!(e, CrwError::Timeout(_));
                        let outcome = classify_outcome(false, false, was_timeout, &attempt_ctx);
                        self.breakers.record_outcome(&host, k, outcome).await;
                        if k == RendererKind::Lightpanda {
                            let _ = self.preferences.record_failure(&host, &err_kind).await;
                        }
                    }
                    if let Some(g) = probe_guard.take() {
                        g.disarm();
                    }
                    last_error = Some(e);
                    continue;
                }
            }
        }
        // Leak-through fallback: every renderer was rejected by the global
        // breaker, but the host itself has no failures recorded. Rather
        // than fail the request outright (which is what made the bench
        // shed ~12% on broad lightpanda outages), give one renderer a
        // single attempt without recording its outcome to the global
        // window. The host tier still records, so a host that's actually
        // broken trips its own breaker on the next attempt.
        // Trigger when every chain attempt failed outright (no thin_result,
        // no Ok return) AND at least one renderer was skipped by the global
        // breaker. Common case: lightpanda runs and errors, chrome gets
        // globally rejected → without leak we'd return error even though
        // chrome's host breaker is clean and would likely succeed.
        //
        // Skip when the request deadline is already (near-)exhausted:
        // entering a renderer with <500ms budget produced 37/128 of the
        // first leak run's failures as "Timeout after 1-2ms" — the
        // attempt cannot succeed and just consumes a CDP connection.
        // (Same reasoning now guards the main ladder loop; see MIN_TIER_BUDGET.)
        if thin_result.is_none()
            && !breaker_skipped.is_empty()
            && !is_user_pinned
            && deadline.remaining() >= MIN_TIER_BUDGET
        {
            for renderer in &renderers_snapshot {
                let kind = renderer_kind_for(renderer.name());
                let trackable = kind.filter(|_| !host.is_empty());
                let Some(k) = trackable else { continue };
                if !breaker_skipped.contains(&k) {
                    continue;
                }
                let permit = self.breakers.try_acquire_host_only(&host, k).await;
                if permit == Permit::Rejected {
                    continue;
                }
                // That acquire awaits; re-check the budget before dispatching so the
                // floor above is not merely advisory (same TOCTOU as the serial loop).
                if deadline.remaining() < MIN_TIER_BUDGET {
                    continue;
                }
                tracing::info!(
                    renderer = renderer.name(),
                    host = %host,
                    "global breaker open, host clean — leaking through one attempt"
                );
                metrics()
                    .render_route_decision_total
                    .with_label_values(&[k.as_str(), "leakThrough"])
                    .inc();
                let attempt_ctx = {
                    let remaining = deadline.remaining();
                    let tier_budget = self.tier_timeouts.get(&k).copied().unwrap_or(remaining);
                    AttemptContext::capture(remaining, tier_budget)
                };
                let res = renderer.fetch(url, headers, wait_for_ms, deadline).await;
                match res {
                    Ok(mut result) => {
                        let text_len = html_body_text_len(&result.html);
                        let is_placeholder = detector::looks_like_loading_placeholder(&result.html);
                        let failed_render = detector::looks_like_failed_render(&result.html);
                        let truncated = result.truncated;
                        let content_ok = text_len >= Self::MIN_RENDERED_TEXT_LEN
                            && !is_placeholder
                            && failed_render.is_none();
                        let outcome = classify_outcome(content_ok, truncated, false, &attempt_ctx);
                        // Record host only — global stays untouched so the
                        // existing trip can finish its cooldown naturally.
                        self.breakers
                            .record_scoped_outcome(&host, k, None, Some(outcome))
                            .await;
                        if content_ok {
                            result.credit_cost = credit_for(k);
                            result.render_decision =
                                Some(RenderDecision::AutoDefault { chosen: k });
                            return Ok(result);
                        }
                        // Thin/placeholder on leak path → fall through to
                        // the normal "no JS renderer" return below.
                        last_error = Some(CrwError::RendererError(format!(
                            "leak attempt on {} returned thin content (text_len={text_len})",
                            renderer.name()
                        )));
                        break;
                    }
                    Err(e) => {
                        let was_timeout = matches!(e, CrwError::Timeout(_));
                        let outcome = classify_outcome(false, false, was_timeout, &attempt_ctx);
                        self.breakers
                            .record_scoped_outcome(&host, k, None, Some(outcome))
                            .await;
                        last_error = Some(e);
                        break;
                    }
                }
            }
        }

        // Phase 2 (latency-qn): gated auto-egress recovery. chrome_proxy was held
        // out of the ladder; fire it ONCE iff the ladder hit a hard block AND the
        // deadline can still absorb a full chrome_proxy attempt (so it never
        // causes a timeout the baseline wouldn't have — the failure mode the
        // naive always-on ladder tier showed: success −2pp, p90 +69%).
        // best-result-wins: never replace usable content with an empty retry.
        if let Some(arm) = auto_egress_arm {
            let kind = RendererKind::ChromeProxy;
            let tier_budget = self
                .tier_timeouts
                .get(&kind)
                .copied()
                .unwrap_or_else(|| std::time::Duration::from_secs(30));
            // `tier_budget` is normally far above MIN_TIER_BUDGET (chrome_proxy defaults
            // to chrome_timeout + 15s), but an operator can configure it lower. Take the
            // stricter of the two so no JS dispatch path can run on a degenerate budget.
            let arm_floor = tier_budget.max(MIN_TIER_BUDGET);
            if saw_hard_block && deadline.remaining() >= arm_floor {
                chain.push(kind);
                let entry = self.pick_proxy_for_url(url);
                let attempt = REQUEST_PROXY
                    .scope(entry, arm.fetch(url, headers, wait_for_ms, deadline))
                    .await;
                match attempt {
                    Ok(r) => {
                        let r_text = html_body_text_len(&r.html);
                        let r_ok = r_text >= Self::MIN_RENDERED_TEXT_LEN
                            && detector::looks_like_failed_render(&r.html).is_none()
                            && !detector::looks_like_loading_placeholder(&r.html);
                        if !host.is_empty() {
                            let outcome = if r_ok {
                                BreakerOutcome::Success
                            } else {
                                BreakerOutcome::RenderError
                            };
                            self.breakers.record_outcome(&host, kind, outcome).await;
                        }
                        // best-result-wins vs the ladder's thin_result: ONLY take
                        // the proxy result if it is content-OK (red line: a thin/
                        // empty proxy result must never turn a baseline Err into an
                        // Ok(empty), nor replace a usable thin_result). Code-review
                        // 🔴#1: gate `None` case on r_ok too, else all-tiers-errored
                        // would ship an empty proxy body as success.
                        let better = r_ok
                            && match &thin_result {
                                Some(prev) => r.html.len() > prev.html.len(),
                                None => true,
                            };
                        if self.latency_breakdown {
                            tracing::info!(
                                target: "latency_breakdown",
                                url, tier = "chrome_proxy",
                                ok = r_ok, consumed = better,
                                "auto_egress fired"
                            );
                        }
                        if better {
                            thin_result = Some(r);
                        }
                    }
                    Err(e) => {
                        if !host.is_empty() {
                            self.breakers
                                .record_outcome(&host, kind, BreakerOutcome::ConnectionError)
                                .await;
                        }
                        if self.latency_breakdown {
                            tracing::info!(
                                target: "latency_breakdown",
                                url, tier = "chrome_proxy", error = %e,
                                "auto_egress fired (error)"
                            );
                        }
                    }
                }
            }
        }

        // Return the best thin result if we have one, otherwise the last error.
        if let Some(mut result) = thin_result {
            // Stamp routing metadata on the soft-failure result too — callers
            // need to know which chain was attempted for debugging.
            if let Some(last) = chain.last().copied() {
                result.credit_cost = credit_for(last);
                result.render_decision = Some(RenderDecision::Failover {
                    chain: chain.clone(),
                    reason: last_failover_reason
                        .clone()
                        .unwrap_or(FailoverErrorKind::Other),
                });
            }
            // When the user hard-pinned a single renderer and it failed thin,
            // failover never ran — surface an actionable hint so callers (SaaS
            // playground, CLI, MCP) can show a banner instead of silently
            // returning broken markdown with `success: true`.
            if is_user_pinned
                && chain.len() == 1
                && let Some(pinned) = chain.first().copied()
            {
                let reason = last_failover_reason
                    .as_ref()
                    .map(|r| r.as_str())
                    .unwrap_or("unknown");
                let hint = format!(
                    "Pinned renderer '{}' returned a failed render ({}). Content may be unreliable. Retry with renderer=\"chrome\" or omit the renderer field for auto-failover.",
                    pinned.as_str(),
                    reason,
                );
                result.warnings.push(hint);
            }
            Ok(result)
        } else {
            Err(last_error
                .unwrap_or_else(|| CrwError::RendererError("No JS renderer available".to_string())))
        }
    }

    /// Check availability of all renderers.
    pub async fn check_health(&self) -> HashMap<String, bool> {
        let mut health = HashMap::new();
        health.insert("http".to_string(), self.http.is_available().await);
        for r in &self.js_renderers {
            health.insert(r.name().to_string(), r.is_available().await);
        }
        health
    }
}

/// Rough estimate of visible text length in an HTML document.
/// Strips tags and collapses whitespace. Used to detect "thin" renders
/// where a renderer returned HTML but failed to execute JavaScript.
fn html_body_text_len(html: &str) -> usize {
    // Extract body content if present, otherwise use entire HTML.
    let body = if let Some(start) = html.find("<body") {
        let start = html[start..].find('>').map(|i| start + i + 1).unwrap_or(0);
        let end = html.find("</body>").unwrap_or(html.len());
        &html[start..end]
    } else {
        html
    };
    // Strip tags crudely.
    let mut in_tag = false;
    let mut text_len = 0;
    let mut prev_ws = true;
    for ch in body.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' {
            in_tag = false;
        } else if !in_tag {
            if ch.is_whitespace() {
                if !prev_ws {
                    text_len += 1;
                    prev_ws = true;
                }
            } else {
                text_len += 1;
                prev_ws = false;
            }
        }
    }
    text_len
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::breaker::BreakerConfig;
    #[cfg(feature = "camoufox")]
    use crw_core::config::CamoufoxEndpoint;
    #[cfg(feature = "cdp")]
    use crw_core::config::CdpEndpoint;
    use std::time::Duration;

    /// Generous deadline used by tests that don't care about budget enforcement.
    fn tdl() -> crw_core::Deadline {
        crw_core::Deadline::now_plus(Duration::from_secs(60))
    }

    fn base_cfg(mode: RendererMode) -> RendererConfig {
        RendererConfig {
            mode,
            ..Default::default()
        }
    }

    #[test]
    fn new_mode_none_ok_no_js_renderers() {
        let cfg = base_cfg(RendererMode::None);
        let r = FallbackRenderer::new(&cfg, "crw-test", None, &StealthConfig::default()).unwrap();
        assert!(r.js_renderer_names().is_empty());
        assert_eq!(r.render_js_default, None);
    }

    #[test]
    fn new_mode_auto_no_endpoints_ok_http_only() {
        let cfg = base_cfg(RendererMode::Auto);
        let r = FallbackRenderer::new(&cfg, "crw-test", None, &StealthConfig::default()).unwrap();
        assert!(r.js_renderer_names().is_empty());
    }

    #[cfg(feature = "cdp")]
    #[test]
    fn new_mode_chrome_without_endpoint_errors() {
        let cfg = base_cfg(RendererMode::Chrome);
        let err =
            FallbackRenderer::new(&cfg, "crw-test", None, &StealthConfig::default()).unwrap_err();
        let msg = err.to_string().to_lowercase();
        assert!(msg.contains("chrome"), "expected chrome in error: {msg}");
        assert!(
            msg.contains("ws_url") || msg.contains("not configured"),
            "expected ws_url hint in error: {msg}"
        );
    }

    #[cfg(feature = "cdp")]
    #[test]
    fn new_mode_chrome_with_endpoint_ok_only_chrome() {
        let cfg = RendererConfig {
            mode: RendererMode::Chrome,
            chrome: Some(CdpEndpoint {
                ws_url: "ws://127.0.0.1:9222/".into(),
            }),
            lightpanda: Some(CdpEndpoint {
                ws_url: "ws://127.0.0.1:9223/".into(),
            }),
            ..Default::default()
        };
        let r = FallbackRenderer::new(&cfg, "crw-test", None, &StealthConfig::default()).unwrap();
        assert_eq!(r.js_renderer_names(), vec!["chrome"]);
    }

    #[cfg(feature = "cdp")]
    #[test]
    fn new_mode_lightpanda_without_endpoint_errors() {
        let cfg = base_cfg(RendererMode::Lightpanda);
        let err =
            FallbackRenderer::new(&cfg, "crw-test", None, &StealthConfig::default()).unwrap_err();
        assert!(err.to_string().to_lowercase().contains("lightpanda"));
    }

    #[cfg(feature = "cdp")]
    #[test]
    fn new_mode_auto_with_both_endpoints_preserves_order() {
        let cfg = RendererConfig {
            mode: RendererMode::Auto,
            lightpanda: Some(CdpEndpoint {
                ws_url: "ws://127.0.0.1:9222/".into(),
            }),
            chrome: Some(CdpEndpoint {
                ws_url: "ws://127.0.0.1:9223/".into(),
            }),
            ..Default::default()
        };
        let r = FallbackRenderer::new(&cfg, "crw-test", None, &StealthConfig::default()).unwrap();
        assert_eq!(r.js_renderer_names(), vec!["lightpanda", "chrome"]);
    }

    #[cfg(feature = "cdp")]
    #[test]
    fn ladder_includes_chrome_proxy_when_configured() {
        let cfg = RendererConfig {
            mode: RendererMode::Auto,
            lightpanda: Some(CdpEndpoint {
                ws_url: "ws://127.0.0.1:9222/".into(),
            }),
            chrome: Some(CdpEndpoint {
                ws_url: "ws://127.0.0.1:9223/".into(),
            }),
            chrome_proxy: Some(CdpEndpoint {
                ws_url: "ws://127.0.0.1:9224/".into(),
            }),
            ..Default::default()
        };
        let r = FallbackRenderer::new(&cfg, "crw-test", None, &StealthConfig::default()).unwrap();
        // chrome_proxy must be the LAST tier — fallback chain tries Chrome
        // direct first and only falls through to the proxy on Chrome failure.
        assert_eq!(
            r.js_renderer_names(),
            vec!["lightpanda", "chrome", "chrome_proxy"]
        );
    }

    #[cfg(feature = "cdp")]
    #[test]
    fn ladder_omits_chrome_proxy_when_not_configured() {
        let cfg = RendererConfig {
            mode: RendererMode::Auto,
            chrome: Some(CdpEndpoint {
                ws_url: "ws://127.0.0.1:9223/".into(),
            }),
            chrome_proxy: None,
            ..Default::default()
        };
        let r = FallbackRenderer::new(&cfg, "crw-test", None, &StealthConfig::default()).unwrap();
        assert!(!r.js_renderer_names().contains(&"chrome_proxy"));
    }

    #[cfg(not(feature = "cdp"))]
    #[test]
    fn new_mode_chrome_errors_without_cdp_feature() {
        let cfg = base_cfg(RendererMode::Chrome);
        let err =
            FallbackRenderer::new(&cfg, "crw-test", None, &StealthConfig::default()).unwrap_err();
        let msg = err.to_string().to_lowercase();
        assert!(msg.contains("cdp"), "expected cdp in error: {msg}");
    }

    #[cfg(feature = "camoufox")]
    fn camoufox_cfg(mode: RendererMode, include_in_auto: bool) -> RendererConfig {
        RendererConfig {
            mode,
            camoufox: Some(CamoufoxEndpoint {
                base_url: "http://127.0.0.1:9377".into(),
                api_key: String::new(),
                include_in_auto,
            }),
            ..Default::default()
        }
    }

    /// Opt-in default: a configured endpoint is CONSTRUCTED (so an explicit
    /// `renderer = "camoufox"` pin can reach it) but does NOT join the auto
    /// ladder when `include_in_auto = false`.
    #[cfg(feature = "camoufox")]
    #[test]
    fn camoufox_constructed_for_pin_but_excluded_from_auto() {
        let cfg = camoufox_cfg(RendererMode::Auto, false);
        let r = FallbackRenderer::new(&cfg, "crw-test", None, &StealthConfig::default()).unwrap();
        assert!(
            r.js_renderer_names().contains(&"camoufox"),
            "configured camoufox must be constructed for pin-reachability"
        );
        assert!(
            !r.camoufox_in_auto,
            "include_in_auto=false must keep camoufox out of the auto ladder"
        );
    }

    #[cfg(feature = "camoufox")]
    #[test]
    fn camoufox_joins_auto_when_include_in_auto_true() {
        let cfg = camoufox_cfg(RendererMode::Auto, true);
        let r = FallbackRenderer::new(&cfg, "crw-test", None, &StealthConfig::default()).unwrap();
        assert!(r.js_renderer_names().contains(&"camoufox"));
        assert!(r.camoufox_in_auto);
    }

    /// `mode = "camoufox"` pins to ONLY camoufox, and must mark it in-auto so a
    /// non-pinned request is not left with zero renderers.
    #[cfg(feature = "camoufox")]
    #[test]
    fn camoufox_pinned_mode_uses_only_camoufox() {
        let cfg = camoufox_cfg(RendererMode::Camoufox, false);
        let r = FallbackRenderer::new(&cfg, "crw-test", None, &StealthConfig::default()).unwrap();
        assert_eq!(r.js_renderer_names(), vec!["camoufox"]);
        assert!(r.camoufox_in_auto);
    }

    #[cfg(feature = "camoufox")]
    #[test]
    fn camoufox_pinned_mode_without_base_url_errors() {
        let cfg = RendererConfig {
            mode: RendererMode::Camoufox,
            camoufox: Some(CamoufoxEndpoint::default()), // empty base_url
            ..Default::default()
        };
        let err =
            FallbackRenderer::new(&cfg, "crw-test", None, &StealthConfig::default()).unwrap_err();
        assert!(err.to_string().to_lowercase().contains("camoufox"));
    }

    #[cfg(feature = "camoufox")]
    #[test]
    fn camoufox_absent_when_not_configured() {
        let cfg = base_cfg(RendererMode::Auto);
        let r = FallbackRenderer::new(&cfg, "crw-test", None, &StealthConfig::default()).unwrap();
        assert!(!r.js_renderer_names().contains(&"camoufox"));
        assert!(!r.camoufox_in_auto);
    }

    #[test]
    fn new_render_js_default_stored() {
        let cfg = RendererConfig {
            mode: RendererMode::None,
            render_js_default: Some(true),
            ..Default::default()
        };
        let r = FallbackRenderer::new(&cfg, "crw-test", None, &StealthConfig::default()).unwrap();
        assert_eq!(r.render_js_default, Some(true));
    }

    /// Mock fetcher for unit-testing dispatch logic without real CDP/HTTP.
    struct MockFetcher {
        name: &'static str,
        behavior: MockBehavior,
    }

    #[derive(Clone)]
    enum MockBehavior {
        Ok(String),
        OkStatus(u16, String),
        Err(String),
    }

    #[async_trait::async_trait]
    impl PageFetcher for MockFetcher {
        async fn fetch(
            &self,
            url: &str,
            _headers: &HashMap<String, String>,
            _wait_for_ms: Option<u64>,
            _deadline: crw_core::Deadline,
        ) -> CrwResult<FetchResult> {
            let (status, html) = match &self.behavior {
                MockBehavior::Ok(html) => (200u16, html.clone()),
                MockBehavior::OkStatus(s, html) => (*s, html.clone()),
                MockBehavior::Err(msg) => return Err(CrwError::RendererError(msg.clone())),
            };
            Ok(FetchResult {
                url: url.to_string(),
                final_url: None,
                status_code: status,
                html,
                content_type: Some("text/html".to_string()),
                raw_bytes: None,
                rendered_with: Some(self.name.to_string()),
                elapsed_ms: 0,
                warning: None,
                render_decision: None,
                credit_cost: 0,
                warnings: Vec::new(),
                truncated: false,
                deadline_exceeded: false,
                captured_responses: Vec::new(),
                screenshot: None,
            })
        }

        fn name(&self) -> &str {
            self.name
        }
        fn supports_js(&self) -> bool {
            true
        }
        async fn is_available(&self) -> bool {
            true
        }
    }

    fn rich_html(marker: &str) -> String {
        format!(
            "<html><body><article>{}{}</article></body></html>",
            marker,
            "x".repeat(200)
        )
    }

    /// Mock that records whether it was invoked. Separate from `MockFetcher` so the
    /// ~12 existing constructor sites stay untouched.
    struct CountingFetcher {
        name: &'static str,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl PageFetcher for CountingFetcher {
        async fn fetch(
            &self,
            url: &str,
            _headers: &HashMap<String, String>,
            _wait_for_ms: Option<u64>,
            _deadline: crw_core::Deadline,
        ) -> CrwResult<FetchResult> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(FetchResult {
                url: url.to_string(),
                final_url: None,
                status_code: 200,
                html: rich_html("rendered"),
                content_type: Some("text/html".to_string()),
                raw_bytes: None,
                rendered_with: Some(self.name.to_string()),
                elapsed_ms: 0,
                warning: None,
                render_decision: None,
                credit_cost: 0,
                warnings: Vec::new(),
                truncated: false,
                deadline_exceeded: false,
                captured_responses: Vec::new(),
                screenshot: None,
            })
        }
        fn name(&self) -> &str {
            self.name
        }
        fn supports_js(&self) -> bool {
            true
        }
        async fn is_available(&self) -> bool {
            true
        }
    }

    /// A degenerate budget must not invoke a JS tier at all (prod: 432 of 536
    /// escalations ran with <50ms, returning a fabricated `Timeout after 5ms`),
    /// and the request must still surface `CrwError::Timeout` so the server keeps
    /// mapping it to 504 rather than a 500 from the `RendererError` tail.
    #[tokio::test]
    async fn degenerate_budget_skips_js_tier_and_preserves_timeout() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mock = Arc::new(CountingFetcher {
            name: "chrome",
            calls: calls.clone(),
        });
        let r = make_renderer_with_mocks(vec![mock]);

        let err = r
            .fetch_with_js(
                "https://example.com",
                &HashMap::new(),
                None,
                None,
                crw_core::Deadline::from_request_ms(0),
            )
            .await
            .expect_err("an exhausted budget must not produce a rendered page");

        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "renderer must be skipped, not invoked with a few milliseconds"
        );
        assert!(
            matches!(err, CrwError::Timeout(_)),
            "must stay a Timeout (504), not RendererError (500); got {err:?}"
        );
    }

    /// Burns most of the budget, then fails — so the NEXT tier lands below the floor.
    struct SlowFailingFetcher {
        name: &'static str,
        burn: Duration,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl PageFetcher for SlowFailingFetcher {
        async fn fetch(
            &self,
            _url: &str,
            _headers: &HashMap<String, String>,
            _wait_for_ms: Option<u64>,
            _deadline: crw_core::Deadline,
        ) -> CrwResult<FetchResult> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            tokio::time::sleep(self.burn).await;
            Err(CrwError::RendererError("anti-bot wall".to_string()))
        }
        fn name(&self) -> &str {
            self.name
        }
        fn supports_js(&self) -> bool {
            true
        }
        async fn is_available(&self) -> bool {
            true
        }
    }

    /// A tier that fails for a real reason, followed by a tier skipped for lack of
    /// budget, must still report Timeout (504) — not the earlier RendererError (500).
    /// The skipped tier would have been invoked, timed out, and overwritten
    /// `last_error`; skipping must not change the status the caller sees. Guards
    /// against reintroducing `get_or_insert_with` here.
    #[tokio::test]
    async fn budget_skip_overrides_an_earlier_renderer_error() {
        let slow_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let slow = Arc::new(SlowFailingFetcher {
            name: "lightpanda",
            burn: Duration::from_millis(1_200),
            calls: slow_calls.clone(),
        });
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let chrome = Arc::new(CountingFetcher {
            name: "chrome",
            calls: calls.clone(),
        });
        let r = make_renderer_with_mocks(vec![slow, chrome]);

        // 1500ms budget: lightpanda is comfortably above the 500ms floor even under
        // CI scheduling jitter, burns 1200ms and errors, leaving ~300ms — chrome is
        // then below the floor and is skipped.
        let err = r
            .fetch_with_js(
                "https://example.com",
                &HashMap::new(),
                None,
                None,
                crw_core::Deadline::from_request_ms(1_500),
            )
            .await
            .expect_err("both tiers must fail");

        assert_eq!(
            slow_calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the first tier must actually run, or this test proves nothing"
        );
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "chrome must be skipped for lack of budget"
        );
        assert!(
            matches!(err, CrwError::Timeout(_)),
            "a budget skip must overwrite the earlier RendererError so the server \
             still maps this to 504; got {err:?}"
        );
    }

    /// When the HTTP tier could not reach the origin at all, that error must win over
    /// the JS tier's generic RendererError. `TargetUnreachable` maps to 422 (the caller
    /// gave us a dead target); `RendererError` falls through to a 500 and reads as "our
    /// server broke". Production emitted 11 such 500s that should have been 422s.
    #[tokio::test]
    async fn unreachable_origin_beats_js_renderer_error() {
        struct Unreachable;
        #[async_trait::async_trait]
        impl PageFetcher for Unreachable {
            async fn fetch(
                &self,
                url: &str,
                _h: &HashMap<String, String>,
                _w: Option<u64>,
                _d: crw_core::Deadline,
            ) -> CrwResult<FetchResult> {
                Err(CrwError::TargetUnreachable(format!(
                    "Could not reach {url}"
                )))
            }
            fn name(&self) -> &str {
                "http"
            }
            fn supports_js(&self) -> bool {
                false
            }
            async fn is_available(&self) -> bool {
                true
            }
        }

        let js = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Err("Navigation failed: net::ERR_SSL".to_string()),
        });
        let mut r = make_renderer_with_mocks(vec![js]);
        r.http = Arc::new(Unreachable);
        r.render_js_default = None; // auto branch

        let err = r
            .fetch(
                "https://dead.example",
                &HashMap::new(),
                None, // render_js: auto
                None, // wait_for_ms
                None, // requested_renderer
                tdl(),
            )
            .await
            .expect_err("both tiers fail");

        assert!(
            matches!(err, CrwError::TargetUnreachable(_)),
            "an unreachable origin must surface as TargetUnreachable (422), not the JS \
             tier's RendererError (500); got {err:?}"
        );
    }

    /// Control: with a healthy budget the same tier IS invoked. Guards against the
    /// floor silently disabling the ladder.
    #[tokio::test]
    async fn healthy_budget_still_invokes_js_tier() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mock = Arc::new(CountingFetcher {
            name: "chrome",
            calls: calls.clone(),
        });
        let r = make_renderer_with_mocks(vec![mock]);

        let res = r
            .fetch_with_js("https://example.com", &HashMap::new(), None, None, tdl())
            .await
            .expect("healthy budget must render");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(res.html.contains("rendered"));
    }

    fn make_renderer_with_mocks(mocks: Vec<Arc<dyn PageFetcher>>) -> FallbackRenderer {
        // Build a real HTTP fetcher (won't be hit when render_js=Some(true)).
        let cfg = base_cfg(RendererMode::None);
        let mut r =
            FallbackRenderer::new(&cfg, "crw-test", None, &StealthConfig::default()).unwrap();
        r.js_renderers = mocks;
        r
    }

    #[tokio::test]
    async fn proxy_active_lightpanda_only_fails_closed() {
        // When a proxy is active but the only JS renderer is lightpanda (which
        // cannot proxy), fetch_with_js must hard-error, never egress direct.
        let lp = Arc::new(MockFetcher {
            name: "lightpanda",
            behavior: MockBehavior::Ok(rich_html("LP-")),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![lp]);
        let entry = Arc::new(crw_core::ProxyEntry::parse("http://p:8080").unwrap());
        // Call fetch_with_js directly to isolate the lightpanda guard from the
        // HTTP pre-fetch (which would otherwise fail against the fake proxy).
        let res = REQUEST_PROXY
            .scope(Some(entry), async {
                r.fetch_with_js(
                    "https://example.com",
                    &HashMap::new(),
                    None,
                    None,
                    crw_core::Deadline::from_request_ms(5000),
                )
                .await
            })
            .await;
        assert!(
            res.is_err(),
            "lightpanda-only + proxy active must fail closed, got {res:?}"
        );
    }

    #[tokio::test]
    async fn proxy_active_prefers_chrome_over_lightpanda() {
        // With a proxy active, lightpanda is skipped and chrome (proxy-capable)
        // serves the request.
        let lp = Arc::new(MockFetcher {
            name: "lightpanda",
            behavior: MockBehavior::Ok(rich_html("LP-")),
        }) as Arc<dyn PageFetcher>;
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Ok(rich_html("CHROME-")),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![lp, chrome]);
        let entry = Arc::new(crw_core::ProxyEntry::parse("http://p:8080").unwrap());
        let res = REQUEST_PROXY
            .scope(Some(entry), async {
                r.fetch_with_js(
                    "https://example.com",
                    &HashMap::new(),
                    None,
                    None,
                    crw_core::Deadline::from_request_ms(5000),
                )
                .await
            })
            .await
            .unwrap();
        assert_eq!(res.rendered_with.as_deref(), Some("chrome"));
    }

    #[tokio::test]
    async fn fetch_with_pinned_renderer_filters_pool() {
        let lp = Arc::new(MockFetcher {
            name: "lightpanda",
            behavior: MockBehavior::Ok(rich_html("LP-")),
        }) as Arc<dyn PageFetcher>;
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Ok(rich_html("CHROME-")),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![lp, chrome]);

        let result = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                Some("chrome"),
                tdl(),
            )
            .await
            .unwrap();
        assert!(result.html.contains("CHROME-"), "expected chrome output");
        assert_eq!(result.rendered_with.as_deref(), Some("chrome"));
    }

    #[tokio::test]
    async fn fetch_with_pinned_renderer_unknown_returns_error() {
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Ok(rich_html("CHROME-")),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![chrome]);

        let err = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                Some("lightpanda"),
                tdl(),
            )
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("lightpanda") && msg.contains("chrome"),
            "expected error to name pinned + available: {msg}"
        );
    }

    #[tokio::test]
    async fn fetch_with_renderer_auto_uses_full_chain() {
        let lp = Arc::new(MockFetcher {
            name: "lightpanda",
            behavior: MockBehavior::Ok(rich_html("LP-")),
        }) as Arc<dyn PageFetcher>;
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Ok(rich_html("CHROME-")),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![lp, chrome]);

        let result = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                Some("auto"),
                tdl(),
            )
            .await
            .unwrap();
        // First renderer in the chain wins when both succeed.
        assert!(result.html.contains("LP-"), "expected lightpanda first");
    }

    #[tokio::test]
    async fn failover_skips_renderer_that_returns_failed_render() {
        // LightPanda returns HTML with a Next.js error boundary marker.
        // The chain must skip it and use Chrome's healthy result.
        let bad_lp_html = format!(
            "<html><body><div id=\"__next-error-0\">{}</div></body></html>",
            "x".repeat(200)
        );
        let lp = Arc::new(MockFetcher {
            name: "lightpanda",
            behavior: MockBehavior::Ok(bad_lp_html),
        }) as Arc<dyn PageFetcher>;
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Ok(rich_html("CHROME-OK")),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![lp, chrome]);

        let result = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                None,
                tdl(),
            )
            .await
            .unwrap();
        assert!(result.html.contains("CHROME-OK"));
        assert_eq!(result.rendered_with.as_deref(), Some("chrome"));
    }

    #[tokio::test]
    async fn failover_surfaces_warning_when_only_failed_render_available() {
        // Only LightPanda is configured and it returns a failed render. The
        // call must succeed (best-effort thin_result fallback) but the warning
        // must name the failure so callers can surface it to the user.
        let bad_lp_html = format!(
            "<html><body><div id=\"__next-error-0\">{}</div></body></html>",
            "x".repeat(200)
        );
        let lp = Arc::new(MockFetcher {
            name: "lightpanda",
            behavior: MockBehavior::Ok(bad_lp_html),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![lp]);

        let result = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                None,
                tdl(),
            )
            .await
            .unwrap();
        let warning = result.warning.expect("expected warning to be set");
        assert!(
            warning.contains("lightpanda") && warning.contains("nextjs_client_error"),
            "warning should name renderer + reason: {warning}"
        );
    }

    #[tokio::test]
    async fn failover_concats_warnings_across_two_failed_renderers() {
        // Both renderers return failed-render HTML. The fallback `thin_result`
        // should carry warnings from BOTH attempts so debugging captures the
        // full chain, not just the first failure.
        let bad_lp_html = format!(
            "<html><body><div id=\"__next-error-0\">{}</div></body></html>",
            "x".repeat(200)
        );
        let bad_chrome_html = format!(
            "<html><body><div id=\"__next_error__\">{}</div></body></html>",
            "y".repeat(200)
        );
        let lp = Arc::new(MockFetcher {
            name: "lightpanda",
            behavior: MockBehavior::Ok(bad_lp_html),
        }) as Arc<dyn PageFetcher>;
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Ok(bad_chrome_html),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![lp, chrome]);

        let result = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                None,
                tdl(),
            )
            .await
            .unwrap();
        let warning = result.warning.expect("expected warning to be set");
        assert!(
            warning.contains("lightpanda") && warning.contains("chrome"),
            "warning should mention both renderers: {warning}"
        );
    }

    #[tokio::test]
    async fn fetch_pinned_renderer_failure_propagates() {
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Err("boom".into()),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![chrome]);

        let err = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                Some("chrome"),
                tdl(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("boom"));
    }

    #[tokio::test]
    async fn auto_promoted_host_tries_chrome_first() {
        // Pre-promote example.com via the preference learner so the loop
        // sorts chrome ahead of lightpanda even though lightpanda was
        // declared first. The first renderer in the executed order wins.
        let lp = Arc::new(MockFetcher {
            name: "lightpanda",
            behavior: MockBehavior::Ok(rich_html("LP-")),
        }) as Arc<dyn PageFetcher>;
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Ok(rich_html("CHROME-")),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![lp, chrome]);

        // Force-promote "example.com" by reaching the failure threshold.
        for _ in 0..3 {
            r.preferences
                .record_failure("example.com", &FailoverErrorKind::NextJsClientError)
                .await;
        }

        let result = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                None,
                tdl(),
            )
            .await
            .unwrap();
        assert!(
            result.html.contains("CHROME-"),
            "promoted host should hit chrome first, got: {}",
            &result.html[..80.min(result.html.len())]
        );
        assert_eq!(result.credit_cost, 1, "every renderer costs 1 credit");
        assert!(matches!(
            result.render_decision,
            Some(RenderDecision::AutoPromoted {
                chosen: RendererKind::Chrome,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn breaker_skipped_renderer_falls_through_to_next() {
        // Trip the per-host breaker for lightpanda, then verify the loop
        // skips it and uses chrome — without ever calling lightpanda.fetch.
        let lp = Arc::new(MockFetcher {
            name: "lightpanda",
            behavior: MockBehavior::Err("would fire if reached".into()),
        }) as Arc<dyn PageFetcher>;
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Ok(rich_html("CHROME-OK")),
        }) as Arc<dyn PageFetcher>;
        let mut r = make_renderer_with_mocks(vec![lp, chrome]);

        // Use a custom breaker config: long cooldown so the breaker can't
        // transition to half-open under parallel test load (the default
        // 5s cooldown was racing against scheduler latency on workspace runs).
        // Threshold/window stay tuned to default: 80 consecutive failures
        // satisfies min_calls=50 and far exceeds failure_rate=0.80.
        let breaker_cfg = BreakerConfig {
            base_cooldown: Duration::from_secs(300),
            max_cooldown: Duration::from_secs(300),
            ..BreakerConfig::default()
        };
        r.breakers = Arc::new(BreakerRegistry::new(breaker_cfg));
        for _ in 0..80 {
            r.breakers
                .record_result("example.com", RendererKind::Lightpanda, false)
                .await;
        }

        let result = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                None,
                tdl(),
            )
            .await
            .unwrap();
        assert!(result.html.contains("CHROME-OK"));
        assert!(matches!(
            result.render_decision,
            Some(RenderDecision::BreakerSkipped {
                skipped: RendererKind::Lightpanda,
                chosen: RendererKind::Chrome
            })
        ));
    }

    #[tokio::test]
    async fn user_pinned_failed_render_emits_warning() {
        // Pin lightpanda. It returns failed-render HTML (Next.js error
        // boundary). Because the user hard-pinned, no failover happens.
        // The thin result must carry an actionable warning so callers can
        // surface it instead of silently returning broken markdown.
        let bad_html = format!(
            "<html><body><div id=\"__next-error-0\">{}</div></body></html>",
            "x".repeat(200)
        );
        let lp = Arc::new(MockFetcher {
            name: "lightpanda",
            behavior: MockBehavior::Ok(bad_html),
        }) as Arc<dyn PageFetcher>;
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Ok(rich_html("CHROME-")),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![lp, chrome]);

        let result = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                Some("lightpanda"),
                tdl(),
            )
            .await
            .unwrap();
        let pin_hint = result
            .warnings
            .iter()
            .find(|w| w.starts_with("Pinned renderer 'lightpanda'"));
        assert!(
            pin_hint.is_some(),
            "expected pin-failure hint in warnings, got: {:?}",
            result.warnings
        );
        let hint = pin_hint.unwrap();
        assert!(
            hint.contains("nextJsClientError"),
            "hint should name camelCase reason: {hint}"
        );
        assert!(
            hint.contains("renderer=\"chrome\""),
            "hint should suggest a fix: {hint}"
        );
        // chain stays single-element because user pinned → no chrome attempt
        assert!(matches!(
            result.render_decision,
            Some(RenderDecision::Failover { ref chain, .. }) if chain.len() == 1
        ));
    }

    #[tokio::test]
    async fn user_pinned_decision_records_credit_and_kind() {
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Ok(rich_html("CHROME-")),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![chrome]);
        let result = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                Some("chrome"),
                tdl(),
            )
            .await
            .unwrap();
        assert_eq!(result.credit_cost, 1);
        assert!(matches!(
            result.render_decision,
            Some(RenderDecision::UserPinned {
                renderer: RendererKind::Chrome
            })
        ));
    }

    #[tokio::test]
    async fn js_tier_escalates_on_403_status() {
        // LightPanda returns 403 with content (e.g. WAF block masked as content).
        // The chain must escalate to Chrome instead of accepting the 403 body.
        let lp = Arc::new(MockFetcher {
            name: "lightpanda",
            behavior: MockBehavior::OkStatus(403, rich_html("BLOCKED-")),
        }) as Arc<dyn PageFetcher>;
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Ok(rich_html("CHROME-")),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![lp, chrome]);

        let result = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                Some("auto"),
                tdl(),
            )
            .await
            .unwrap();
        assert!(
            result.html.contains("CHROME-"),
            "expected chrome output after lightpanda 403"
        );
        assert_eq!(result.status_code, 200);
    }

    #[tokio::test]
    async fn js_tier_escalates_on_vendor_block_with_200() {
        // LightPanda returns 200 with a Cloudflare challenge page. The chain
        // must escalate even though the status code is "successful".
        let cf_html = format!(
            "<html><head><script src=\"/cdn-cgi/challenge-platform/h/g/orchestrate/chl_page/v1\"></script></head><body>{}</body></html>",
            "x".repeat(200)
        );
        let lp = Arc::new(MockFetcher {
            name: "lightpanda",
            behavior: MockBehavior::Ok(cf_html),
        }) as Arc<dyn PageFetcher>;
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Ok(rich_html("CHROME-")),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![lp, chrome]);

        let result = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                Some("auto"),
                tdl(),
            )
            .await
            .unwrap();
        assert!(
            result.html.contains("CHROME-"),
            "expected chrome output after lightpanda vendor block"
        );
    }

    #[tokio::test]
    async fn js_tier_accepts_200_clean_response() {
        // Regression: a clean 200 from the first renderer must still be
        // accepted — no false escalation triggered by the new gates.
        let lp = Arc::new(MockFetcher {
            name: "lightpanda",
            behavior: MockBehavior::Ok(rich_html("LP-CLEAN-")),
        }) as Arc<dyn PageFetcher>;
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Ok(rich_html("CHROME-")),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![lp, chrome]);

        let result = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                Some("auto"),
                tdl(),
            )
            .await
            .unwrap();
        assert!(result.html.contains("LP-CLEAN-"));
        assert_eq!(result.status_code, 200);
    }

    /// A page the lightweight `detector` heuristics pass but the
    /// `crw_extract::antibot` classifier flags — a Reddit-class WAF block
    /// ("blocked by network security") served with HTTP 200.
    fn network_security_block_html() -> String {
        format!(
            "<html><body><article>You've been blocked by network security.{}</article></body></html>",
            "x".repeat(200)
        )
    }

    #[tokio::test]
    async fn js_tier_escalates_to_chrome_proxy_on_antibot_block() {
        // lightpanda + chrome both return a 200 WAF block the detector
        // misses; only the residential chrome_proxy tier clears it.
        let lp = Arc::new(MockFetcher {
            name: "lightpanda",
            behavior: MockBehavior::Ok(network_security_block_html()),
        }) as Arc<dyn PageFetcher>;
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Ok(network_security_block_html()),
        }) as Arc<dyn PageFetcher>;
        let chrome_proxy = Arc::new(MockFetcher {
            name: "chrome_proxy",
            behavior: MockBehavior::Ok(rich_html("PROXY-")),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![lp, chrome, chrome_proxy]);

        let result = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                Some("auto"),
                tdl(),
            )
            .await
            .unwrap();
        assert!(
            result.html.contains("PROXY-"),
            "expected chrome_proxy output after antibot block"
        );
        assert_eq!(
            result.render_decision,
            Some(RenderDecision::Failover {
                chain: vec![
                    RendererKind::Lightpanda,
                    RendererKind::Chrome,
                    RendererKind::ChromeProxy,
                ],
                reason: FailoverErrorKind::AntibotBlock,
            })
        );
    }

    #[tokio::test]
    async fn antibot_block_returns_as_success_when_escalation_disabled() {
        // Kill switch: escalate_in_failover = false → classify() still runs
        // for telemetry, but the block page is returned as success with no
        // escalation. Proves the gate is wired correctly.
        let lp = Arc::new(MockFetcher {
            name: "lightpanda",
            behavior: MockBehavior::Ok(network_security_block_html()),
        }) as Arc<dyn PageFetcher>;
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Ok(rich_html("CHROME-")),
        }) as Arc<dyn PageFetcher>;
        let mut r = make_renderer_with_mocks(vec![lp, chrome]);
        r.antibot.escalate_in_failover = false;

        let result = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                Some("auto"),
                tdl(),
            )
            .await
            .unwrap();
        assert!(
            result.html.contains("network security"),
            "block page should be returned as-is when escalation is disabled"
        );
        assert_eq!(result.rendered_with.as_deref(), Some("lightpanda"));
    }

    #[tokio::test]
    async fn promoted_host_escalates_chrome_to_chrome_proxy_not_lightpanda() {
        // After host promotion the preference sort must place chrome_proxy
        // immediately after chrome — a chrome block escalates straight to
        // the residential tier, never back down to lightpanda.
        let lp = Arc::new(MockFetcher {
            name: "lightpanda",
            behavior: MockBehavior::Ok(rich_html("LP-")),
        }) as Arc<dyn PageFetcher>;
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Ok(network_security_block_html()),
        }) as Arc<dyn PageFetcher>;
        let chrome_proxy = Arc::new(MockFetcher {
            name: "chrome_proxy",
            behavior: MockBehavior::Ok(rich_html("PROXY-")),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![lp, chrome, chrome_proxy]);

        // Force-promote "example.com" so the loop sorts chrome first.
        for _ in 0..3 {
            r.preferences
                .record_failure("example.com", &FailoverErrorKind::NextJsClientError)
                .await;
        }

        let result = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                None,
                tdl(),
            )
            .await
            .unwrap();
        assert!(
            result.html.contains("PROXY-"),
            "expected chrome_proxy output"
        );
        assert_eq!(
            result.render_decision,
            Some(RenderDecision::Failover {
                chain: vec![RendererKind::Chrome, RendererKind::ChromeProxy],
                reason: FailoverErrorKind::AntibotBlock,
            }),
            "chrome must escalate straight to chrome_proxy, skipping lightpanda"
        );
    }
}
