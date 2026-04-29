use async_trait::async_trait;
use crw_core::error::{CrwError, CrwResult};
use crw_core::types::FetchResult;
use std::collections::HashMap;
use std::time::Instant;

use crate::traits::PageFetcher;

/// Maximum response body size (10 MB) to prevent memory exhaustion.
const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024;
/// TCP connect timeout for HTTP requests.
const HTTP_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// Overall request timeout for HTTP requests.
const HTTP_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Stealth headers injected when stealth mode is enabled.
/// These mimic a real browser's default request headers.
const STEALTH_ACCEPT: &str =
    "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8";
/// Chrome 131 client hint — kept in sync with the UA strings in BUILTIN_UA_POOL.
const STEALTH_SEC_CH_UA: &str =
    r#""Google Chrome";v="131", "Chromium";v="131", "Not_A Brand";v="24""#;

/// Simple HTTP fetcher using reqwest. No JS rendering.
pub struct HttpFetcher {
    client: reqwest::Client,
    inject_stealth_headers: bool,
}

impl HttpFetcher {
    pub fn new(user_agent: &str, proxy: Option<&str>, inject_stealth_headers: bool) -> Self {
        let mut builder = reqwest::Client::builder()
            .user_agent(user_agent)
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .timeout(HTTP_REQUEST_TIMEOUT)
            .redirect(crw_core::url_safety::safe_redirect_policy());

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
        Self {
            client,
            inject_stealth_headers,
        }
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

        // Inject stealth headers before user-supplied headers so users can override.
        if self.inject_stealth_headers {
            req = req
                .header("Accept", STEALTH_ACCEPT)
                .header("Accept-Language", "en-US,en;q=0.9")
                .header("Sec-Ch-Ua", STEALTH_SEC_CH_UA)
                .header("Sec-Ch-Ua-Mobile", "?0")
                .header("Sec-Ch-Ua-Platform", "\"Windows\"")
                .header("Sec-Fetch-Dest", "document")
                .header("Sec-Fetch-Mode", "navigate")
                .header("Sec-Fetch-Site", "none")
                .header("Sec-Fetch-User", "?1")
                .header("Upgrade-Insecure-Requests", "1")
                .header("Priority", "u=0, i");
        }

        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.send().await.map_err(|e| {
            if e.is_connect() {
                CrwError::TargetUnreachable(format!("Could not reach {url}: {e}"))
            } else {
                CrwError::HttpError(e.to_string())
            }
        })?;
        let status = resp.status().as_u16();

        // Check content-length before downloading
        if let Some(len) = resp.content_length()
            && len as usize > MAX_RESPONSE_BYTES
        {
            return Err(CrwError::HttpError(format!(
                "Response too large: {len} bytes (max {MAX_RESPONSE_BYTES})"
            )));
        }

        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.split(';').next().unwrap_or(s).trim().to_lowercase());

        let cf_mitigated = resp
            .headers()
            .get("cf-mitigated")
            .and_then(|v| v.to_str().ok())
            .map(crate::detector::is_cloudflare_mitigated_header)
            .unwrap_or(false);

        let is_pdf = content_type.as_deref() == Some("application/pdf");

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

        let (html, raw_bytes) = if is_pdf {
            (String::new(), Some(bytes.to_vec()))
        } else {
            (String::from_utf8_lossy(&bytes).into_owned(), None)
        };

        Ok(FetchResult {
            url: url.to_string(),
            status_code: status,
            html,
            content_type,
            raw_bytes,
            rendered_with: if is_pdf {
                Some("pdf".to_string())
            } else {
                Some("http".to_string())
            },
            elapsed_ms: start.elapsed().as_millis() as u64,
            warning: if cf_mitigated {
                Some("cloudflare_mitigated".to_string())
            } else {
                None
            },
            render_decision: None,
            credit_cost: 0,
            warnings: if cf_mitigated {
                vec!["cf-mitigated header indicates Cloudflare challenge or block".to_string()]
            } else {
                Vec::new()
            },
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
