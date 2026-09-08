//! Doctor subcommand: read-only diagnostics.
//!
//! `crw doctor` never installs, starts, repairs, or rewrites anything. When
//! something is wrong it points at `crw setup` or names the exact fix.
//!
//! Exit codes: 0 when every check passed (warnings are informational and
//! still exit 0), 1 when a required check failed, 2 on a usage/config
//! ambiguity (e.g. both a cloud API and a local renderer are configured and
//! `--target` was not given).

use super::diag::{self, CheckResult, Target};
use crate::teardown::CmdError;
use clap::Args;
use crw_core::config::AppConfig;
use std::time::Duration;

#[derive(Args)]
pub struct DoctorArgs {
    /// Which backend to diagnose. Auto-resolved from config when omitted: a
    /// fresh install (no cloud API configured) resolves to `local`; a
    /// configured `[client].api_url` with no local renderer resolves to
    /// `cloud`. Configuring both requires this flag. `mcp` is never
    /// auto-resolved.
    #[arg(long, value_enum)]
    pub target: Option<Target>,

    /// Emit machine-readable JSON instead of human-readable text.
    #[arg(long)]
    pub json: bool,
}

const REACHABILITY_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn run(args: DoctorArgs) -> Result<(), CmdError> {
    let mut checks = vec![binary_check()];

    let config = match AppConfig::load() {
        Ok(c) => {
            checks.push(CheckResult::pass(
                "config.load",
                "config parsed successfully",
            ));
            c
        }
        Err(e) => {
            checks.push(CheckResult::fail(
                "config.load",
                format!("config failed to parse: {e}"),
                "fix the TOML syntax at the reported path, or run `crw setup --reset` to start clean",
            ));
            AppConfig::default()
        }
    };
    checks.push(config_source_check());
    checks.push(CheckResult::skip(
        "config.unknown-fields",
        "not surfaced: the config loader merges layers (config.default.toml, the user \
         config, CRW_* env vars) without tracking per-field provenance, so an unknown or \
         renamed key is silently dropped rather than reported. Diff config.toml against \
         config.default.toml, or run `crw setup --reset`, if you suspect stale keys.",
    ));
    checks.push(fs_writable_check());

    let target = match diag::resolve_target(&config, args.target) {
        Ok(t) => t,
        Err(msg) => {
            if args.json {
                let body = serde_json::json!({ "error": msg, "exitCode": 2 });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&body).expect("error body always serializes")
                );
            } else {
                eprintln!("error: {msg}");
            }
            return Err(CmdError::code_only(2));
        }
    };

    match target {
        Target::Local => checks.extend(run_local_checks(&config).await),
        Target::Cloud => checks.extend(run_cloud_checks(&config).await),
        Target::Mcp => {
            checks.push(mcp_mode_check(&config));
            if config.client.api_url.is_some() {
                checks.extend(run_cloud_checks(&config).await);
            } else {
                checks.extend(run_local_checks(&config).await);
            }
        }
    }

    let exit_code = diag::print_report("crw doctor", target, &checks, args.json);
    if exit_code == 0 {
        Ok(())
    } else {
        Err(CmdError::code_only(exit_code))
    }
}

fn binary_check() -> CheckResult {
    let mut features = Vec::new();
    if cfg!(feature = "serve") {
        features.push("serve");
    }
    if cfg!(feature = "mcp-embedded") {
        features.push("mcp-embedded");
    }
    if cfg!(feature = "browse") {
        features.push("browse");
    }
    if cfg!(feature = "camoufox") {
        features.push("camoufox");
    }
    if cfg!(feature = "cloak") {
        features.push("cloak");
    }
    CheckResult::pass(
        "binary.build",
        format!(
            "crw {} (features: {})",
            env!("CARGO_PKG_VERSION"),
            if features.is_empty() {
                "none".to_string()
            } else {
                features.join(", ")
            }
        ),
    )
}

fn config_source_check() -> CheckResult {
    let layers = diag::describe_config_layers();
    if layers.is_empty() {
        CheckResult::pass(
            "config.source",
            "no config file found; using built-in defaults (CRW_* env vars still apply)",
        )
    } else {
        CheckResult::pass(
            "config.source",
            format!(
                "merged, lowest to highest precedence: {} (CRW_* env vars win over all of these)",
                layers.join(" < ")
            ),
        )
    }
}

/// Probes the nearest existing ancestor of the user config directory, plus
/// the system temp dir, for write access, without creating anything crw
/// doesn't already create on its own (`crw setup` creates the config dir;
/// this only checks whether it *could*).
fn fs_writable_check() -> CheckResult {
    fn nearest_existing_ancestor(path: &std::path::Path) -> std::path::PathBuf {
        let mut p = path.to_path_buf();
        loop {
            if p.exists() {
                return p;
            }
            match p.parent() {
                Some(parent) => p = parent.to_path_buf(),
                None => return p,
            }
        }
    }

    let mut candidates = vec![std::env::temp_dir()];
    if let Some(dir) =
        crw_core::config::user_config_path().and_then(|p| p.parent().map(nearest_existing_ancestor))
    {
        candidates.push(dir);
    }
    candidates.dedup();

    let mut unwritable = Vec::new();
    let mut writable = Vec::new();
    for dir in &candidates {
        let probe = dir.join(format!(".crw-doctor-probe-{}", std::process::id()));
        match std::fs::write(&probe, b"ok") {
            Ok(()) => {
                let _ = std::fs::remove_file(&probe);
                writable.push(dir.display().to_string());
            }
            Err(e) => unwritable.push(format!("{} ({e})", dir.display())),
        }
    }

    if unwritable.is_empty() {
        CheckResult::pass(
            "fs.cache-writable",
            format!("writable: {}", writable.join(", ")),
        )
    } else {
        CheckResult::fail(
            "fs.cache-writable",
            format!("not writable: {}", unwritable.join("; ")),
            "fix directory permissions, or point CRW_USER_CONFIG_DIR / TMPDIR at a writable path",
        )
    }
}

async fn run_local_checks(config: &AppConfig) -> Vec<CheckResult> {
    let mut out = Vec::new();

    #[cfg(feature = "mcp-embedded")]
    {
        let local_browsers = LocalBrowsers::detect();
        match diag::local_capabilities(config.clone()).await {
            Ok(caps) => {
                out.push(renderer_connectivity_check(config, &local_browsers).await);
                out.push(screenshot_check(&caps, &local_browsers));
                out.push(search_check(config, &caps).await);
                out.push(llm_check(&caps));
            }
            Err(e) => {
                out.push(CheckResult::fail(
                    "capabilities.snapshot",
                    format!("could not evaluate local capabilities: {e}"),
                    "check the config errors above (proxy/renderer/search) and re-run",
                ));
            }
        }
    }
    #[cfg(not(feature = "mcp-embedded"))]
    {
        out.push(CheckResult::skip(
            "capabilities.snapshot",
            "this binary was built without the embedded engine (--no-default-features); \
             renderer/search/LLM/screenshot facts are unavailable",
        ));
    }

    out.push(proxy_parse_check(config));
    out.push(proxy_reachability_check(config).await);
    out.push(listen_port_check(config).await);
    out
}

#[cfg(feature = "mcp-embedded")]
struct LocalBrowsers {
    chrome: Option<String>,
    lightpanda: Option<String>,
}

#[cfg(feature = "mcp-embedded")]
impl LocalBrowsers {
    fn detect() -> Self {
        Self {
            chrome: crw_renderer::browser::detect_local_chrome(),
            lightpanda: crw_renderer::browser::detect_local_lightpanda(),
        }
    }

    fn names(&self) -> Vec<&'static str> {
        let mut names = Vec::new();
        if self.lightpanda.is_some() {
            names.push("LightPanda");
        }
        if self.chrome.is_some() {
            names.push("Chrome/Chromium");
        }
        names
    }
}

#[cfg(feature = "mcp-embedded")]
async fn renderer_connectivity_check(
    config: &AppConfig,
    local_browsers: &LocalBrowsers,
) -> CheckResult {
    // camoufox is a REST sidecar (`base_url`), not a CDP websocket
    // (`ws_url`) like the other four, but `diag::tcp_reachable` only ever
    // does a host:port TCP connect regardless of scheme, so the same probe
    // applies once the right field is picked. An empty `base_url` mirrors
    // `RendererConfig::camoufox_in_ladder`'s own "configured" check: the
    // section can be present with a blank URL.
    let endpoints: Vec<(&str, &str)> = [
        (
            "chrome",
            config.renderer.chrome.as_ref().map(|e| e.ws_url.as_str()),
        ),
        (
            "lightpanda",
            config
                .renderer
                .lightpanda
                .as_ref()
                .map(|e| e.ws_url.as_str()),
        ),
        (
            "chrome_proxy",
            config
                .renderer
                .chrome_proxy
                .as_ref()
                .map(|e| e.ws_url.as_str()),
        ),
        (
            "playwright",
            config
                .renderer
                .playwright
                .as_ref()
                .map(|e| e.ws_url.as_str()),
        ),
        (
            "camoufox",
            config
                .renderer
                .camoufox
                .as_ref()
                .map(|e| e.base_url.as_str())
                .filter(|url| !url.trim().is_empty()),
        ),
    ]
    .into_iter()
    .filter_map(|(name, ep)| ep.map(|url| (name, url)))
    .collect();

    if endpoints.is_empty() {
        let discovered = local_browsers.names();
        if !discovered.is_empty() {
            return CheckResult::pass(
                "renderer.connectivity",
                format!(
                    "runtime-discovered local browser(s): {} (CLI/MCP auto-spawn available)",
                    discovered.join(", ")
                ),
            );
        }
        return CheckResult::warn(
            "renderer.connectivity",
            "no JS renderer configured; running HTTP-only (JS-heavy pages return thin content)",
            "optional: run `crw setup --local` to add a JS renderer for SPA-heavy sites",
        );
    }

    let mut unreachable = Vec::new();
    for (name, ws_url) in &endpoints {
        if diag::tcp_reachable(ws_url, REACHABILITY_TIMEOUT)
            .await
            .is_err()
        {
            unreachable.push(*name);
        }
    }

    if unreachable.is_empty() {
        let names: Vec<&str> = endpoints.iter().map(|(n, _)| *n).collect();
        CheckResult::pass(
            "renderer.connectivity",
            format!("reachable: {}", names.join(", ")),
        )
    } else {
        let discovered = local_browsers.names();
        let local_fallback = if discovered.is_empty() {
            String::new()
        } else {
            format!(
                "; local {} detected for CLI/MCP auto-spawn",
                discovered.join(", ")
            )
        };
        CheckResult::warn(
            "renderer.connectivity",
            format!(
                "configured endpoint(s) unreachable: {}{}",
                unreachable.join(", "),
                local_fallback
            ),
            "start the renderer container(s) (e.g. `docker compose up chrome`), or fix the \
             configured endpoint URL; local CLI auto-spawn does not require a CDP URL",
        )
    }
}

#[cfg(feature = "mcp-embedded")]
fn screenshot_check(
    caps: &crw_server::routes::capabilities::Capabilities,
    local_browsers: &LocalBrowsers,
) -> CheckResult {
    if caps.screenshot.supported || local_browsers.chrome.is_some() {
        let message = if caps.screenshot.supported {
            "screenshot capture available from configured renderer"
        } else {
            "screenshot capture available through runtime-discovered Chrome/Chromium"
        };
        CheckResult::pass("renderer.screenshot", message)
    } else {
        CheckResult::warn(
            "renderer.screenshot",
            "screenshot capture unavailable (needs a chrome / chrome_proxy / playwright renderer)",
            "configure a screenshot-capable renderer, e.g. via `crw setup --local`",
        )
    }
}

#[cfg(feature = "mcp-embedded")]
async fn search_check(
    config: &AppConfig,
    caps: &crw_server::routes::capabilities::Capabilities,
) -> CheckResult {
    if !caps.search.supported {
        return CheckResult::warn(
            "search.reachability",
            "search backend not configured; `crw search` and the crw_search MCP tool are disabled",
            "run `crw setup --local` to boot a local search backend, or set \
             search.search_backend_url in config.toml",
        );
    }
    let Some(url) = config.search.resolve_backend_url() else {
        return CheckResult::warn(
            "search.reachability",
            "search backend not configured",
            "run `crw setup --local` to boot a local search backend",
        );
    };
    match diag::tcp_reachable(url, REACHABILITY_TIMEOUT).await {
        Ok(()) => CheckResult::pass("search.reachability", "search backend reachable"),
        Err(()) => CheckResult::warn(
            "search.reachability",
            "search backend configured but unreachable",
            "start the search backend container, or check search.search_backend_url",
        ),
    }
}

#[cfg(feature = "mcp-embedded")]
fn llm_check(caps: &crw_server::routes::capabilities::Capabilities) -> CheckResult {
    if caps.llm.server_key_configured {
        CheckResult::pass(
            "llm.configured",
            "server-side LLM key configured (value redacted)",
        )
    } else {
        CheckResult::warn(
            "llm.configured",
            "no server-side LLM key configured",
            "--summary/--extract need either a server key (`crw setup`) or a per-request \
             --llm-key (BYOK)",
        )
    }
}

fn proxy_parse_check(config: &AppConfig) -> CheckResult {
    match crw_core::ProxyRotator::build(
        &config.crawler.proxy_list,
        config.crawler.proxy.as_deref(),
        config.crawler.proxy_rotation,
    ) {
        Ok(Some(rotator)) => CheckResult::pass(
            "proxy.parse",
            format!(
                "{} proxy entr{} parsed",
                rotator.len(),
                if rotator.len() == 1 { "y" } else { "ies" }
            ),
        ),
        Ok(None) => CheckResult::pass("proxy.parse", "no proxy configured (direct egress)"),
        Err(_e) => {
            // `crw_core::ProxyEntry::parse` redacts userinfo, but this report
            // gets pasted into support threads: withhold the value entirely and
            // report the shape of the problem instead.
            let count = if config.crawler.proxy_list.is_empty() {
                1
            } else {
                config.crawler.proxy_list.len()
            };
            CheckResult::fail(
                "proxy.parse",
                format!(
                    "proxy config is invalid: one of {count} configured entr{} failed to \
                     parse (malformed URL, unsupported scheme, or missing host); value \
                     withheld, it may contain credentials",
                    if count == 1 { "y" } else { "ies" }
                ),
                "fix crawler.proxy / crawler.proxy_list in config.toml",
            )
        }
    }
}

async fn proxy_reachability_check(config: &AppConfig) -> CheckResult {
    let raw = config
        .crawler
        .proxy_list
        .first()
        .map(String::as_str)
        .or(config.crawler.proxy.as_deref());
    let Some(raw) = raw else {
        return CheckResult::skip("proxy.reachability", "no proxy configured");
    };
    let redacted = diag::redact_url(raw);
    match diag::tcp_reachable(raw, REACHABILITY_TIMEOUT).await {
        Ok(()) => CheckResult::pass("proxy.reachability", format!("reachable: {redacted}")),
        Err(()) => CheckResult::warn(
            "proxy.reachability",
            format!("unreachable: {redacted}"),
            "check the proxy host/port and credentials, or your egress network",
        ),
    }
}

async fn listen_port_check(config: &AppConfig) -> CheckResult {
    let addr = format!("{}:{}", config.server.host, config.server.port);
    // `config.server.host` can be a hostname, in which case `bind` performs
    // name resolution with no built-in bound, unlike every other probe here.
    match tokio::time::timeout(REACHABILITY_TIMEOUT, tokio::net::TcpListener::bind(&addr)).await {
        Ok(Ok(_)) => CheckResult::pass("server.listen-port", format!("{addr} is free")),
        Ok(Err(e)) => CheckResult::warn(
            "server.listen-port",
            format!("{addr} unavailable: {e}"),
            "stop the process using the port, or pass --port to `crw serve`",
        ),
        Err(_) => CheckResult::warn(
            "server.listen-port",
            format!(
                "{addr} could not be checked: timed out after {}s",
                REACHABILITY_TIMEOUT.as_secs()
            ),
            "check that server.host resolves, or pass --port to `crw serve`",
        ),
    }
}

async fn run_cloud_checks(config: &AppConfig) -> Vec<CheckResult> {
    let mut out = Vec::new();

    let Some(api_url) = config.client.api_url.as_deref() else {
        out.push(CheckResult::warn(
            "remote.api-url",
            "no remote API URL configured",
            "set CRW_API_URL or run `crw setup --cloud`",
        ));
        out.push(CheckResult::skip(
            "cloud.reachability",
            "no [client].api_url configured",
        ));
        return out;
    };

    let managed = diag::is_managed_api_url(api_url);
    out.push(match (managed, config.client.api_key.is_some()) {
        (_, true) => CheckResult::pass("remote.api-key", "API key configured (value redacted)"),
        (true, false) => CheckResult::warn(
            "remote.api-key",
            "no Cloud API key configured",
            "run `crw setup --cloud --api-key <key>` (get one at fastcrw.com/dashboard)",
        ),
        (false, false) => CheckResult::pass(
            "remote.api-key",
            "no API key configured (valid for an unauthenticated self-hosted server)",
        ),
    });
    out.push(cloud_reachability_check(api_url, config.client.api_key.as_deref()).await);
    out
}

async fn cloud_reachability_check(api_url: &str, api_key: Option<&str>) -> CheckResult {
    let redacted = diag::redact_url(api_url);
    let url = format!("{}/v1/capabilities", api_url.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(REACHABILITY_TIMEOUT)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return CheckResult::fail(
                "cloud.reachability",
                format!("could not build an HTTP client: {e}"),
                "check local TLS/cert setup",
            );
        }
    };
    let mut request = client.get(&url);
    if let Some(api_key) = api_key {
        request = request.bearer_auth(api_key);
    }
    match request.send().await {
        Ok(resp) if resp.status().is_success() => CheckResult::pass(
            "cloud.reachability",
            format!("{redacted} reachable ({})", resp.status()),
        ),
        Ok(resp) => CheckResult::fail(
            "cloud.reachability",
            format!("{redacted} responded {}", resp.status()),
            "check [client].api_url and api_key, or https://fastcrw.com/dashboard for an outage",
        ),
        Err(e) => CheckResult::fail(
            "cloud.reachability",
            format!("{redacted} unreachable: {e}"),
            "check your network and [client].api_url",
        ),
    }
}

fn mcp_mode_check(config: &AppConfig) -> CheckResult {
    match &config.client.api_url {
        Some(url) => CheckResult::pass(
            "mcp.mode",
            format!("proxy mode via {}", diag::redact_url(url)),
        ),
        None => CheckResult::pass("mcp.mode", "embedded mode (local engine)"),
    }
}

#[cfg(test)]
mod tests {
    use super::diag::Status;
    use super::*;

    /// A malformed proxy entry that still carries userinfo must never reach
    /// the report `--json` and human output are both built from, in either
    /// field checked here (message, fix).
    #[test]
    fn proxy_parse_check_withholds_a_malformed_entry_with_userinfo() {
        let mut config = AppConfig::default();
        config.crawler.proxy_list = vec!["ftp://leakeduser:leakedpass@bad.example:21".to_string()];

        let result = proxy_parse_check(&config);

        assert_eq!(result.status, Status::Fail);
        assert!(!result.message.contains("leakeduser"));
        assert!(!result.message.contains("leakedpass"));
        assert!(!result.message.contains("bad.example"));
        if let Some(fix) = &result.fix {
            assert!(!fix.contains("leakeduser"));
            assert!(!fix.contains("leakedpass"));
        }

        // The JSON `--json` output serializes this same struct, so proving
        // the struct is clean proves both output modes are clean.
        let json = serde_json::to_string(&result).expect("CheckResult always serializes");
        assert!(!json.contains("leakeduser"));
        assert!(!json.contains("leakedpass"));
    }

    #[test]
    fn proxy_parse_check_passes_with_no_proxy_configured() {
        let config = AppConfig::default();
        let result = proxy_parse_check(&config);
        assert_eq!(result.status, Status::Pass);
        assert!(result.message.contains("no proxy configured"));
        assert!(result.fix.is_none());
    }

    #[test]
    fn proxy_parse_check_passes_with_a_single_valid_proxy() {
        let mut config = AppConfig::default();
        config.crawler.proxy = Some("http://127.0.0.1:8888".to_string());
        let result = proxy_parse_check(&config);
        assert_eq!(result.status, Status::Pass);
        assert!(
            result.message.contains("1 proxy entry parsed"),
            "got: {}",
            result.message
        );
    }

    #[test]
    fn proxy_parse_check_pluralizes_entries_correctly() {
        let mut config = AppConfig::default();
        config.crawler.proxy_list = vec![
            "http://127.0.0.1:8888".to_string(),
            "http://127.0.0.1:8889".to_string(),
        ];
        let result = proxy_parse_check(&config);
        assert_eq!(result.status, Status::Pass);
        assert!(
            result.message.contains("2 proxy entries parsed"),
            "got: {}",
            result.message
        );
    }

    #[test]
    fn proxy_parse_check_reports_count_from_proxy_list_when_multiple_malformed() {
        let mut config = AppConfig::default();
        config.crawler.proxy_list = vec![
            "not a url at all".to_string(),
            "also not a url".to_string(),
            "still not a url".to_string(),
        ];
        let result = proxy_parse_check(&config);
        assert_eq!(result.status, Status::Fail);
        assert!(
            result.message.contains("one of 3 configured entries"),
            "got: {}",
            result.message
        );
    }

    #[test]
    fn proxy_parse_check_reports_singular_when_single_scalar_proxy_malformed() {
        let mut config = AppConfig::default();
        config.crawler.proxy = Some("not a url at all".to_string());
        let result = proxy_parse_check(&config);
        assert_eq!(result.status, Status::Fail);
        assert!(
            result.message.contains("one of 1 configured entry"),
            "got: {}",
            result.message
        );
        let fix = result.fix.unwrap();
        assert!(fix.contains("crawler.proxy"));
    }

    #[test]
    fn binary_check_reports_pass_with_version_and_features() {
        let result = binary_check();
        assert_eq!(result.id, "binary.build");
        assert_eq!(result.status, Status::Pass);
        assert!(result.message.contains(env!("CARGO_PKG_VERSION")));
        assert!(result.message.contains("features:"));
    }

    #[test]
    fn mcp_mode_check_reports_proxy_mode_with_redacted_url() {
        let mut config = AppConfig::default();
        config.client.api_url = Some("https://user:pass@api.fastcrw.com".to_string());
        let result = mcp_mode_check(&config);
        assert_eq!(result.status, Status::Pass);
        assert!(result.message.contains("proxy mode via"));
        assert!(
            !result.message.contains("pass"),
            "credentials must be redacted"
        );
    }

    #[test]
    fn mcp_mode_check_reports_embedded_mode_when_no_api_url() {
        let config = AppConfig::default();
        let result = mcp_mode_check(&config);
        assert_eq!(result.status, Status::Pass);
        assert!(result.message.contains("embedded mode"));
    }

    #[tokio::test]
    async fn listen_port_check_passes_for_a_free_ephemeral_port() {
        // Bind to port 0 to let the OS hand back a free port, then release it
        // immediately so the check can bind it itself.
        let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);

        let mut config = AppConfig::default();
        config.server.host = "127.0.0.1".to_string();
        config.server.port = port;
        let result = listen_port_check(&config).await;
        assert_eq!(result.status, Status::Pass);
        assert!(result.message.contains("is free"));
    }

    #[tokio::test]
    async fn listen_port_check_warns_when_port_already_bound() {
        let held = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = held.local_addr().unwrap().port();

        let mut config = AppConfig::default();
        config.server.host = "127.0.0.1".to_string();
        config.server.port = port;
        let result = listen_port_check(&config).await;
        assert_eq!(result.status, Status::Warn);
        assert!(result.message.contains("unavailable"));
        drop(held);
    }

    #[tokio::test]
    async fn proxy_reachability_check_skips_when_unconfigured() {
        let config = AppConfig::default();
        let result = proxy_reachability_check(&config).await;
        assert_eq!(result.status, Status::Skip);
    }

    #[tokio::test]
    async fn proxy_reachability_check_passes_against_a_local_listener() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        // Keep the listener alive for the duration of the connect attempt.
        let _accept_task = tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        let mut config = AppConfig::default();
        config.crawler.proxy = Some(format!("http://127.0.0.1:{port}"));
        let result = proxy_reachability_check(&config).await;
        assert_eq!(result.status, Status::Pass);
        assert!(result.message.contains("reachable"));
    }

    #[tokio::test]
    async fn proxy_reachability_check_warns_on_a_closed_port() {
        // Bind then drop to get a port nothing is listening on.
        let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);

        let mut config = AppConfig::default();
        config.crawler.proxy = Some(format!("http://127.0.0.1:{port}"));
        let result = proxy_reachability_check(&config).await;
        assert_eq!(result.status, Status::Warn);
        assert!(result.message.contains("unreachable"));
    }

    #[tokio::test]
    async fn proxy_reachability_check_prefers_proxy_list_head_over_scalar_proxy() {
        let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);

        let mut config = AppConfig::default();
        // Scalar `proxy` would report a different (bogus, non-parsing) host;
        // `proxy_list`'s first entry must win so the redacted message reflects
        // the entry actually probed.
        config.crawler.proxy = Some("http://this-should-be-ignored.invalid:1".to_string());
        config.crawler.proxy_list = vec![format!("http://127.0.0.1:{port}")];
        let result = proxy_reachability_check(&config).await;
        assert!(!result.message.contains("this-should-be-ignored"));
    }
}
