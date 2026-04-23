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
//! let result = renderer.fetch("https://example.com", &HashMap::new(), None, None).await?;
//! println!("status: {}", result.status_code);
//! # Ok(())
//! # }
//! ```

#[cfg(feature = "auto-browser")]
pub mod browser;
#[cfg(feature = "cdp")]
pub mod cdp;
#[cfg(feature = "cdp")]
pub mod cdp_conn;
pub mod detector;
pub mod http_only;
pub mod traits;

use crw_core::config::{BUILTIN_UA_POOL, RendererConfig, RendererMode, StealthConfig};
use crw_core::error::{CrwError, CrwResult};
use crw_core::types::{FetchResult, resolve_render_js};
use std::collections::HashMap;
use std::sync::Arc;
use traits::PageFetcher;

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

/// Composite renderer that tries multiple backends in order.
pub struct FallbackRenderer {
    http: Arc<dyn PageFetcher>,
    js_renderers: Vec<Arc<dyn PageFetcher>>,
    /// Global default for `render_js` when a request doesn't specify one.
    render_js_default: Option<bool>,
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
        let http = Arc::new(http_only::HttpFetcher::new(
            &effective_ua,
            proxy,
            inject_headers,
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
            });
        }

        #[cfg(feature = "cdp")]
        {
            let want = |m: RendererMode| -> bool {
                matches!(config.mode, RendererMode::Auto) || config.mode == m
            };

            if want(RendererMode::Lightpanda) {
                if let Some(lp) = &config.lightpanda {
                    js_renderers.push(Arc::new(cdp::CdpRenderer::new(
                        "lightpanda",
                        &lp.ws_url,
                        config.page_timeout_ms,
                        config.pool_size,
                    )));
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
                    js_renderers.push(Arc::new(cdp::CdpRenderer::new(
                        "playwright",
                        &pw.ws_url,
                        config.page_timeout_ms,
                        config.pool_size,
                    )));
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
                    js_renderers.push(Arc::new(cdp::CdpRenderer::new(
                        "chrome",
                        &ch.ws_url,
                        config.page_timeout_ms,
                        config.pool_size,
                    )));
                } else if matches!(config.mode, RendererMode::Chrome) {
                    return Err(CrwError::ConfigError(
                        "renderer.mode = \"chrome\" but [renderer.chrome] ws_url is not configured"
                            .into(),
                    ));
                }
            }
        }

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
        })
    }

    /// Names of the configured JS renderers in fallback order.
    /// Used for startup logs and tests — does not leak internal types.
    pub fn js_renderer_names(&self) -> Vec<&str> {
        self.js_renderers.iter().map(|r| r.name()).collect()
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
    ) -> CrwResult<FetchResult> {
        let effective = resolve_render_js(render_js, self.render_js_default);
        tracing::debug!(
            url,
            request_render_js = ?render_js,
            default_render_js = ?self.render_js_default,
            effective_render_js = ?effective,
            "FallbackRenderer::fetch dispatching"
        );
        match effective {
            Some(false) => self.http.fetch(url, headers, None).await,
            Some(true) => {
                // Fetch via HTTP first to check content type — PDFs can't be JS-rendered.
                let http_result = self.http.fetch(url, headers, None).await?;
                if http_result.content_type.as_deref() == Some("application/pdf") {
                    return Ok(http_result);
                }

                if self.js_renderers.is_empty() {
                    tracing::warn!(
                        url,
                        "JS rendering requested but no renderer available — falling back to HTTP"
                    );
                    let mut result = http_result;
                    result.rendered_with = Some("http_only_fallback".to_string());
                    result.warning = Some("JS rendering was requested but no renderer is available. Content was fetched via HTTP only.".to_string());
                    Ok(result)
                } else {
                    self.fetch_with_js(url, headers, wait_for_ms).await
                }
            }
            None => {
                let result = self.http.fetch(url, headers, None).await?;

                // PDFs don't need JS rendering — return immediately.
                if result.content_type.as_deref() == Some("application/pdf") {
                    return Ok(result);
                }

                let needs_js = detector::needs_js_rendering(&result.html);
                let is_blocked = Self::looks_like_challenge(&result.html);
                let is_auth_blocked = matches!(result.status_code, 401 | 403);

                if !self.js_renderers.is_empty() && (needs_js || is_blocked || is_auth_blocked) {
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
                    } else {
                        tracing::info!(url, "SPA shell detected, retrying with JS renderer");
                    }
                    match self.fetch_with_js(url, headers, wait_for_ms).await {
                        Ok(js_result) => Ok(js_result),
                        Err(e) => {
                            tracing::warn!("JS rendering failed, falling back to HTTP result: {e}");
                            Ok(result)
                        }
                    }
                } else {
                    Ok(result)
                }
            }
        }
    }

    /// Quick check if HTML looks like an anti-bot challenge/interstitial page.
    fn looks_like_challenge(html: &str) -> bool {
        if html.len() > 50_000 {
            return false;
        }
        let lower = html.to_lowercase();
        lower.contains("just a moment")
            || lower.contains("cf-browser-verification")
            || lower.contains("cf-challenge-running")
            || lower.contains("challenge-platform")
            || (lower.contains("attention required") && lower.contains("cloudflare"))
    }

    /// Minimum body text length for a JS-rendered result to be considered
    /// successful. If the rendered page has less visible text than this, the
    /// next renderer in the chain is tried.
    const MIN_RENDERED_TEXT_LEN: usize = 50;

    async fn fetch_with_js(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        wait_for_ms: Option<u64>,
    ) -> CrwResult<FetchResult> {
        let mut last_error = None;
        let mut thin_result: Option<FetchResult> = None;
        for renderer in &self.js_renderers {
            match renderer.fetch(url, headers, wait_for_ms).await {
                Ok(result) => {
                    let text_len = html_body_text_len(&result.html);
                    let is_placeholder = detector::looks_like_loading_placeholder(&result.html);
                    if text_len >= Self::MIN_RENDERED_TEXT_LEN && !is_placeholder {
                        return Ok(result);
                    }
                    tracing::info!(
                        renderer = renderer.name(),
                        text_len,
                        is_placeholder,
                        "JS renderer returned thin/placeholder content, trying next renderer"
                    );
                    if thin_result.is_none() {
                        thin_result = Some(result);
                    }
                }
                Err(e) => {
                    tracing::warn!(renderer = renderer.name(), "JS renderer failed: {e}");
                    last_error = Some(e);
                    continue;
                }
            }
        }
        // Return the best thin result if we have one, otherwise the last error.
        if let Some(result) = thin_result {
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
    use crw_core::config::CdpEndpoint;

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

    #[cfg(not(feature = "cdp"))]
    #[test]
    fn new_mode_chrome_errors_without_cdp_feature() {
        let cfg = base_cfg(RendererMode::Chrome);
        let err =
            FallbackRenderer::new(&cfg, "crw-test", None, &StealthConfig::default()).unwrap_err();
        let msg = err.to_string().to_lowercase();
        assert!(msg.contains("cdp"), "expected cdp in error: {msg}");
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
}
