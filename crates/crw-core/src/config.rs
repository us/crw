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

#[derive(Debug, Clone, Deserialize)]
pub struct RendererConfig {
    #[serde(default = "default_renderer_mode")]
    pub mode: String,
    #[serde(default = "default_page_timeout")]
    pub page_timeout_ms: u64,
    #[serde(default = "default_pool_size")]
    pub pool_size: usize,
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
            mode: default_renderer_mode(),
            page_timeout_ms: default_page_timeout(),
            pool_size: default_pool_size(),
            lightpanda: None,
            playwright: None,
            chrome: None,
        }
    }
}

fn default_renderer_mode() -> String {
    "auto".into()
}
fn default_page_timeout() -> u64 {
    30000
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
    /// HTTP/HTTPS proxy URL (e.g. "http://proxy:8080")
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
    "CRW/0.1".into()
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

    #[test]
    fn env_var_overrides_toml_defaults() {
        // Set env vars that should override config.default.toml values.
        unsafe {
            std::env::set_var("CRW_SERVER__PORT", "4444");
            std::env::set_var("CRW_RENDERER__LIGHTPANDA__WS_URL", "ws://test:9999/");
        }

        let cfg = AppConfig::load().unwrap();

        // Clean up.
        unsafe {
            std::env::remove_var("CRW_SERVER__PORT");
            std::env::remove_var("CRW_RENDERER__LIGHTPANDA__WS_URL");
        }

        assert_eq!(cfg.server.port, 4444, "env var should override server.port");
        assert_eq!(
            cfg.renderer.lightpanda.as_ref().unwrap().ws_url,
            "ws://test:9999/",
            "env var should override renderer.lightpanda.ws_url"
        );
    }
}
