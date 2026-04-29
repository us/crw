use async_trait::async_trait;
use crw_core::error::{CrwError, CrwResult};
use crw_core::types::FetchResult;
use std::collections::HashMap;
use std::time::Instant;

use crate::traits::PageFetcher;

/// Maximum response body size (50 MB) to prevent memory exhaustion. The
/// previous 10 MB cap rejected legitimate large reports/PDFs (bench had a
/// ~12 MB PDF mis-flagged as 502). 50 MB is generous enough for almost any
/// document while still bounding memory use.
const MAX_RESPONSE_BYTES: usize = 50 * 1024 * 1024;
/// TCP connect timeout for HTTP requests.
const HTTP_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// Overall request timeout for HTTP requests.
const HTTP_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// One retry on transient errors. GET is idempotent so a single retry is safe;
/// origins frequently emit 502/503/504 under brief overload and connect/timeout
/// errors are often DNS or TCP races that resolve on the next attempt.
const HTTP_MAX_RETRIES: u32 = 1;
/// Backoff before the retry attempt. Short — we are inside the request path
/// and the upstream timeout is 30s, so we cannot afford long sleeps.
const HTTP_RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_millis(250);

/// Returns true if a `reqwest::Error` represents a transient failure worth
/// retrying. Connect failures often succeed on the next attempt; `is_timeout`
/// covers cases where the first connection stalled before the body started
/// arriving. `is_request()` is intentionally NOT included — it also fires on
/// permanent builder/config errors that no amount of retrying will fix.
fn is_retriable_error(e: &reqwest::Error) -> bool {
    e.is_connect() || e.is_timeout()
}

/// Returns true if a response status warrants one retry. Limited to the
/// canonical transient gateway/origin signals — 5xx errors that are not
/// retriable (501, 505) are excluded so we don't waste time on permanent
/// upstream misconfigurations.
fn is_retriable_status(status: u16) -> bool {
    matches!(status, 502..=504)
}

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

        // Build a fresh, fully-decorated request for each attempt. Closure
        // captures `self`, `url`, and `headers`; called once per attempt so
        // every retry sends an independent (yet identical) request.
        let build_request = || {
            let mut req = self.client.get(url);
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
            req
        };

        // Single-retry loop on transient errors / 502-503-504. GET is
        // idempotent so this is safe.
        let mut attempt: u32 = 0;
        let resp = loop {
            match build_request().send().await {
                Ok(r) if attempt < HTTP_MAX_RETRIES && is_retriable_status(r.status().as_u16()) => {
                    tracing::debug!(
                        "HTTP {} from {url}, retrying (attempt {})",
                        r.status(),
                        attempt + 1
                    );
                    drop(r);
                    attempt += 1;
                    tokio::time::sleep(HTTP_RETRY_BACKOFF).await;
                }
                Ok(r) => break r,
                Err(e) if attempt < HTTP_MAX_RETRIES && is_retriable_error(&e) => {
                    tracing::debug!(
                        "transient HTTP error to {url} ({e}), retrying (attempt {})",
                        attempt + 1
                    );
                    attempt += 1;
                    tokio::time::sleep(HTTP_RETRY_BACKOFF).await;
                }
                Err(e) => {
                    return Err(if e.is_connect() {
                        CrwError::TargetUnreachable(format!("Could not reach {url}: {e}"))
                    } else {
                        CrwError::HttpError(e.to_string())
                    });
                }
            }
        };
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
