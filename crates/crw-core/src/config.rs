use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
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
}

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self {
            default_format: default_format(),
            only_main_content: true,
            llm: None,
        }
    }
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
