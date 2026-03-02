use async_trait::async_trait;
use crw_core::error::{CrwError, CrwResult};
use crw_core::types::FetchResult;
use std::collections::HashMap;
use std::time::Instant;

use crate::traits::PageFetcher;

/// Maximum response body size (10 MB) to prevent memory exhaustion.
const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024;

/// Simple HTTP fetcher using reqwest. No JS rendering.
pub struct HttpFetcher {
    client: reqwest::Client,
}

impl HttpFetcher {
    pub fn new(user_agent: &str, proxy: Option<&str>) -> Self {
        let mut builder = reqwest::Client::builder()
            .user_agent(user_agent)
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(10));

        if let Some(proxy_url) = proxy {
            match reqwest::Proxy::all(proxy_url) {
                Ok(p) => builder = builder.proxy(p),
                Err(e) => tracing::warn!("Invalid proxy URL '{}': {}", proxy_url, e),
            }
        }

        let client = match builder.build() {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Failed to build HTTP client: {e}, using default");
                reqwest::Client::new()
            }
        };
        Self { client }
    }
}

#[async_trait]
impl PageFetcher for HttpFetcher {
    async fn fetch(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        _wait_for_ms: Option<u64>,
    ) -> CrwResult<FetchResult> {
        let start = Instant::now();

        let mut req = self.client.get(url);
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.send().await.map_err(|e| CrwError::HttpError(e.to_string()))?;
        let status = resp.status().as_u16();

        // Check content-length before downloading
        if let Some(len) = resp.content_length() {
            if len as usize > MAX_RESPONSE_BYTES {
                return Err(CrwError::HttpError(format!(
                    "Response too large: {len} bytes (max {MAX_RESPONSE_BYTES})"
                )));
            }
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| CrwError::HttpError(e.to_string()))?;

        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(CrwError::HttpError(format!(
                "Response too large: {} bytes (max {MAX_RESPONSE_BYTES})",
                bytes.len()
            )));
        }

        let html = String::from_utf8_lossy(&bytes).into_owned();

        Ok(FetchResult {
            url: url.to_string(),
            status_code: status,
            html,
            rendered_with: None,
            elapsed_ms: start.elapsed().as_millis() as u64,
        })
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
