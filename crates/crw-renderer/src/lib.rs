#[cfg(feature = "cdp")]
pub mod cdp;
pub mod detector;
pub mod http_only;
pub mod traits;

use crw_core::config::RendererConfig;
use crw_core::error::{CrwError, CrwResult};
use crw_core::types::FetchResult;
use std::collections::HashMap;
use std::sync::Arc;
use traits::PageFetcher;

/// Composite renderer that tries multiple backends in order.
pub struct FallbackRenderer {
    http: Arc<dyn PageFetcher>,
    js_renderers: Vec<Arc<dyn PageFetcher>>,
}

impl FallbackRenderer {
    pub fn new(config: &RendererConfig, user_agent: &str, proxy: Option<&str>) -> Self {
        let http = Arc::new(http_only::HttpFetcher::new(user_agent, proxy)) as Arc<dyn PageFetcher>;

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
                )));
            }
            if let Some(pw) = &config.playwright {
                js_renderers.push(Arc::new(cdp::CdpRenderer::new(
                    "playwright",
                    &pw.ws_url,
                    config.page_timeout_ms,
                )));
            }
            if let Some(ch) = &config.chrome {
                js_renderers.push(Arc::new(cdp::CdpRenderer::new(
                    "chrome",
                    &ch.ws_url,
                    config.page_timeout_ms,
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
    pub async fn fetch(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        render_js: Option<bool>,
        wait_for_ms: Option<u64>,
    ) -> CrwResult<FetchResult> {
        match render_js {
            Some(false) => self.http.fetch(url, headers, None).await,
            Some(true) => self.fetch_with_js(url, headers, wait_for_ms).await,
            None => {
                let result = self.http.fetch(url, headers, None).await?;
                if !self.js_renderers.is_empty() && detector::needs_js_rendering(&result.html) {
                    tracing::info!(url, "SPA shell detected, retrying with JS renderer");
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

    async fn fetch_with_js(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        wait_for_ms: Option<u64>,
    ) -> CrwResult<FetchResult> {
        for renderer in &self.js_renderers {
            match renderer.fetch(url, headers, wait_for_ms).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    tracing::warn!(renderer = renderer.name(), "JS renderer failed: {e}");
                    continue;
                }
            }
        }
        Err(CrwError::RendererError(
            "No JS renderer available".to_string(),
        ))
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
