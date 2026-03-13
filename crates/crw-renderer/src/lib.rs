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
//! let renderer = FallbackRenderer::new(&config, "crw/0.1", None, &stealth);
//! let result = renderer.fetch("https://example.com", &HashMap::new(), None, None).await?;
//! println!("status: {}", result.status_code);
//! # Ok(())
//! # }
//! ```

#[cfg(feature = "cdp")]
pub mod cdp;
pub mod detector;
pub mod http_only;
pub mod traits;

use crw_core::config::{BUILTIN_UA_POOL, RendererConfig, StealthConfig};
use crw_core::error::{CrwError, CrwResult};
use crw_core::types::FetchResult;
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
            return stealth.user_agents[rand::random::<usize>() % stealth.user_agents.len()]
                .clone();
        };
        pool[rand::random::<usize>() % pool.len()].to_string()
    } else {
        default_ua.to_string()
    }
}

/// Composite renderer that tries multiple backends in order.
pub struct FallbackRenderer {
    http: Arc<dyn PageFetcher>,
    js_renderers: Vec<Arc<dyn PageFetcher>>,
}

impl FallbackRenderer {
    pub fn new(
        config: &RendererConfig,
        user_agent: &str,
        proxy: Option<&str>,
        stealth: &StealthConfig,
    ) -> Self {
        let effective_ua = pick_ua(user_agent, stealth);
        let inject_headers = stealth.enabled && stealth.inject_headers;
        let http = Arc::new(http_only::HttpFetcher::new(
            &effective_ua,
            proxy,
            inject_headers,
        )) as Arc<dyn PageFetcher>;

        #[allow(unused_mut)]
        let mut js_renderers: Vec<Arc<dyn PageFetcher>> = Vec::new();

        if config.mode == "none" {
            return Self { http, js_renderers };
        }

        #[cfg(feature = "cdp")]
        {
            if let Some(lp) = &config.lightpanda {
                js_renderers.push(Arc::new(cdp::CdpRenderer::new(
                    "lightpanda",
                    &lp.ws_url,
                    config.page_timeout_ms,
                    config.pool_size,
                )));
            }
            if let Some(pw) = &config.playwright {
                js_renderers.push(Arc::new(cdp::CdpRenderer::new(
                    "playwright",
                    &pw.ws_url,
                    config.page_timeout_ms,
                    config.pool_size,
                )));
            }
            if let Some(ch) = &config.chrome {
                js_renderers.push(Arc::new(cdp::CdpRenderer::new(
                    "chrome",
                    &ch.ws_url,
                    config.page_timeout_ms,
                    config.pool_size,
                )));
            }
        }

        #[cfg(not(feature = "cdp"))]
        if config.lightpanda.is_some() || config.playwright.is_some() || config.chrome.is_some() {
            tracing::warn!(
                "CDP renderers configured but 'cdp' feature not enabled. JS rendering disabled."
            );
        }

        Self { http, js_renderers }
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
        match render_js {
            Some(false) => self.http.fetch(url, headers, None).await,
            Some(true) => {
                if self.js_renderers.is_empty() {
                    tracing::warn!(
                        url,
                        "JS rendering requested but no renderer available — falling back to HTTP"
                    );
                    let mut result = self.http.fetch(url, headers, None).await?;
                    result.rendered_with = Some("http_only_fallback".to_string());
                    result.warning = Some("JS rendering was requested but no renderer is available. Content was fetched via HTTP only.".to_string());
                    Ok(result)
                } else {
                    self.fetch_with_js(url, headers, wait_for_ms).await
                }
            }
            None => {
                let result = self.http.fetch(url, headers, None).await?;

                let needs_js = detector::needs_js_rendering(&result.html);
                let is_blocked = Self::looks_like_challenge(&result.html);

                if !self.js_renderers.is_empty() && (needs_js || is_blocked) {
                    if is_blocked {
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

    async fn fetch_with_js(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        wait_for_ms: Option<u64>,
    ) -> CrwResult<FetchResult> {
        let mut last_error = None;
        for renderer in &self.js_renderers {
            match renderer.fetch(url, headers, wait_for_ms).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    tracing::warn!(renderer = renderer.name(), "JS renderer failed: {e}");
                    last_error = Some(e);
                    continue;
                }
            }
        }
        Err(last_error
            .unwrap_or_else(|| CrwError::RendererError("No JS renderer available".to_string())))
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
