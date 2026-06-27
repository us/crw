use async_trait::async_trait;
use crw_core::Deadline;
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

/// Returns true if a response status means the origin is rate-limiting the
/// host's egress IP — a signal that a *different* egress IP (proxy) may clear
/// it. 429 = Too Many Requests (the explicit rate-limit signal). Retried ONCE
/// through the configured proxy when armed; every other status is untouched.
fn is_ratelimit_status(status: u16) -> bool {
    matches!(status, 429)
}

/// Is `CRW_HTTP_TLS_RELAXED_FALLBACK` enabled? When on, a fetch that fails TLS
/// certificate verification is retried ONCE with verification disabled (small
/// orgs frequently misconfigure their chain — e.g. a CA cert served as the leaf,
/// or an expired/self-signed cert — yet the content is perfectly fetchable).
/// Cert-errors-only; every other failure mode keeps strict verification.
fn tls_relaxed_fallback_enabled() -> bool {
    std::env::var("CRW_HTTP_TLS_RELAXED_FALLBACK")
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            v == "true" || v == "1" || v == "yes"
        })
        .unwrap_or(false)
}

/// The proxy URL to retry through when an origin rate-limits the host's egress
/// IP (`CRW_HTTP_RATELIMIT_PROXY_URL`, e.g. `http://user:pass@gateway:port`).
/// When set, a fetch that returns 429 is retried ONCE through this proxy — a
/// different egress IP usually clears the limit, so the engine no longer stalls
/// behind a single shared IP when a huge proxy pool is available. Unset (or
/// empty) = behavior identical to before (no proxy retry). SSRF protection is
/// unaffected (it runs on the resolved target URL, not the proxy hop).
fn ratelimit_proxy_url() -> Option<String> {
    std::env::var("CRW_HTTP_RATELIMIT_PROXY_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Returns true if a `reqwest::Error` (or anything in its source chain) is a TLS
/// certificate verification failure — the ONLY error class the relaxed-TLS
/// fallback should react to. Detected by message (rustls/openssl surface these
/// as opaque connect errors, so there is no typed predicate to match on).
fn is_cert_error(e: &reqwest::Error) -> bool {
    let mut src: Option<&(dyn std::error::Error + 'static)> = Some(e);
    while let Some(s) = src {
        let m = s.to_string().to_ascii_lowercase();
        if m.contains("certificate")
            || m.contains("peerfailedverification")
            || m.contains("sslconnecterror")
            || m.contains("invalid peer cert")
            || m.contains("certusedasend")
            || m.contains("cert verify")
            || m.contains("tls handshake")
            || (m.contains("ssl") && (m.contains("verif") || m.contains("cert")))
        {
            return true;
        }
        src = s.source();
    }
    false
}

/// Stealth headers injected when stealth mode is enabled.
/// These mimic a real browser's default request headers.
const STEALTH_ACCEPT: &str =
    "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8";
/// Chrome 150 client hint — kept in sync with the UA strings in BUILTIN_UA_POOL.
const STEALTH_SEC_CH_UA: &str =
    r#""Google Chrome";v="150", "Chromium";v="150", "Not_A Brand";v="24""#;

/// Build a configured reqwest client, optionally routed through `proxy`.
///
/// **Strict**: a malformed proxy URL or a client build failure is a hard error
/// — we never silently fall back to a direct (no-proxy) client, which would leak
/// the host's real IP. Reached via [`HttpFetcher::with_timeout`] (infallible —
/// callers pre-validate) and [`HttpFetcher::with_proxy`] (fail-closed per-request
/// path for config rotation + BYOP, where the error path IS reachable).
fn build_client(
    user_agent: &str,
    proxy: Option<&str>,
    request_timeout: std::time::Duration,
    relaxed_tls: bool,
) -> CrwResult<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .user_agent(user_agent)
        .connect_timeout(HTTP_CONNECT_TIMEOUT)
        .timeout(request_timeout)
        .redirect(crw_core::url_safety::safe_redirect_policy());

    // Relaxed client used ONLY as a cert-error fallback (see `is_cert_error`):
    // disable cert + hostname verification so a broken chain / expired / self-
    // signed cert no longer blocks an otherwise-fetchable page. SSRF protection
    // is unaffected (it runs on the resolved URL, not the TLS layer).
    if relaxed_tls {
        builder = builder
            .danger_accept_invalid_certs(true)
            .danger_accept_invalid_hostnames(true);
    }

    if let Some(proxy_url) = proxy {
        let p = reqwest::Proxy::all(proxy_url)
            .map_err(|e| CrwError::ConfigError(format!("invalid proxy URL '{proxy_url}': {e}")))?;
        builder = builder.proxy(p);
    }

    builder
        .build()
        .map_err(|e| CrwError::ConfigError(format!("failed to build HTTP client: {e}")))
}

/// Simple HTTP fetcher using reqwest. No JS rendering.
pub struct HttpFetcher {
    client: reqwest::Client,
    /// Cert-verification-disabled client, built only when
    /// `CRW_HTTP_TLS_RELAXED_FALLBACK` is on. Used solely to retry a fetch that
    /// failed strict TLS verification (`is_cert_error`); `None` keeps behavior
    /// identical to before.
    relaxed_client: Option<reqwest::Client>,
    /// Proxy-routed client, built only when `CRW_HTTP_RATELIMIT_PROXY_URL` is
    /// set. Used solely to retry a fetch the origin rate-limited (429) through a
    /// different egress IP (`is_ratelimit_status`); `None` keeps behavior
    /// identical to before.
    ratelimit_proxy_client: Option<reqwest::Client>,
    inject_stealth_headers: bool,
}

impl HttpFetcher {
    pub fn new(user_agent: &str, proxy: Option<&str>, inject_stealth_headers: bool) -> Self {
        Self::with_timeout(
            user_agent,
            proxy,
            inject_stealth_headers,
            HTTP_REQUEST_TIMEOUT,
        )
    }

    /// Same as [`Self::new`] but with a caller-supplied request timeout.
    /// Used by `FallbackRenderer` to honor `RendererConfig::http_timeout()`.
    ///
    /// Infallible: callers that pass a `proxy` must pre-validate it (the renderer
    /// does, via `ProxyEntry::parse`, so this never silently falls back for a
    /// configured proxy). The strict per-request path is [`Self::with_proxy`].
    pub fn with_timeout(
        user_agent: &str,
        proxy: Option<&str>,
        inject_stealth_headers: bool,
        request_timeout: std::time::Duration,
    ) -> Self {
        let client = build_client(user_agent, proxy, request_timeout, false).unwrap_or_else(|e| {
            tracing::error!("{e}, using default client");
            reqwest::Client::new()
        });
        let relaxed_client = if tls_relaxed_fallback_enabled() {
            build_client(user_agent, proxy, request_timeout, true).ok()
        } else {
            None
        };
        let ratelimit_proxy_client = ratelimit_proxy_url().and_then(|purl| {
            build_client(user_agent, Some(purl.as_str()), request_timeout, false).ok()
        });
        Self {
            client,
            relaxed_client,
            ratelimit_proxy_client,
            inject_stealth_headers,
        }
    }

    /// Build a fetcher bound to a specific proxy. **Fail-closed**: a bad proxy
    /// URL or client build failure is a hard error — never a silent direct
    /// (no-proxy) client. Used for per-request proxy egress (config rotation +
    /// BYOP) so the HTTP path provably uses the selected proxy.
    pub fn with_proxy(
        user_agent: &str,
        proxy_url: &str,
        inject_stealth_headers: bool,
        request_timeout: std::time::Duration,
    ) -> CrwResult<Self> {
        let client = build_client(user_agent, Some(proxy_url), request_timeout, false)?;
        let relaxed_client = if tls_relaxed_fallback_enabled() {
            build_client(user_agent, Some(proxy_url), request_timeout, true).ok()
        } else {
            None
        };
        let ratelimit_proxy_client = ratelimit_proxy_url().and_then(|purl| {
            build_client(user_agent, Some(purl.as_str()), request_timeout, false).ok()
        });
        Ok(Self {
            client,
            relaxed_client,
            ratelimit_proxy_client,
            inject_stealth_headers,
        })
    }
}

#[async_trait]
impl PageFetcher for HttpFetcher {
    async fn fetch(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        _wait_for_ms: Option<u64>,
        deadline: Deadline,
    ) -> CrwResult<FetchResult> {
        if deadline.expired() {
            return Err(CrwError::HttpError(format!(
                "deadline expired before HTTP fetch of {url}"
            )));
        }
        let start = Instant::now();

        // Build a fresh, fully-decorated request for each attempt. Closure
        // captures `self`, `url`, and `headers`; called once per attempt so
        // every retry sends an independent (yet identical) request.
        let build_request = |client: &reqwest::Client| {
            let mut req = client.get(url);
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
        // idempotent so this is safe. Each attempt is bounded by the caller's
        // remaining deadline so the request cannot exceed the overall budget.
        let mut attempt: u32 = 0;
        let mut use_relaxed = false;
        let mut use_proxy = false;
        let resp = loop {
            let remaining = deadline.remaining();
            if remaining.is_zero() {
                // Already past the budget — report elapsed-since-call so the
                // message reads "Timeout after Xms" instead of a useless 0.
                return Err(CrwError::Timeout(
                    (start.elapsed().as_millis().max(1)) as u64,
                ));
            }
            // On the cert-error fallback path use the verification-disabled
            // client; otherwise the strict client.
            let active_client = if use_proxy {
                self.ratelimit_proxy_client.as_ref().unwrap_or(&self.client)
            } else if use_relaxed {
                self.relaxed_client.as_ref().unwrap_or(&self.client)
            } else {
                &self.client
            };
            let send_fut = build_request(active_client).send();
            let send_result = tokio::time::timeout(remaining, send_fut).await;
            match send_result {
                Err(_) => {
                    return Err(CrwError::Timeout(remaining.as_millis() as u64));
                }
                Ok(Ok(r))
                    if attempt < HTTP_MAX_RETRIES && is_retriable_status(r.status().as_u16()) =>
                {
                    tracing::debug!(
                        "HTTP {} from {url}, retrying (attempt {})",
                        r.status(),
                        attempt + 1
                    );
                    drop(r);
                    attempt += 1;
                    let backoff = HTTP_RETRY_BACKOFF.min(deadline.remaining());
                    if !backoff.is_zero() {
                        tokio::time::sleep(backoff).await;
                    }
                }
                // Origin rate-limited our egress IP (429) and a fallback proxy
                // is armed: retry ONCE through the proxy (a different egress IP
                // usually clears the limit). Not a transient retry — does not
                // consume the retry budget. Placed before the success arm so the
                // 429 is not returned before the proxy is tried.
                Ok(Ok(r))
                    if !use_proxy
                        && self.ratelimit_proxy_client.is_some()
                        && is_ratelimit_status(r.status().as_u16()) =>
                {
                    tracing::warn!(
                        "HTTP {} from {url} (origin rate-limited); retrying once via proxy (ratelimit_bypassed)",
                        r.status()
                    );
                    drop(r);
                    use_proxy = true;
                }
                Ok(Ok(r)) => break r,
                // TLS cert verification failed and relaxed-TLS fallback is armed:
                // swap to the cert-disabled client and retry once. NOT a transient
                // retry — does not consume the retry budget or back off. Placed
                // before the generic retry arm because cert failures are
                // `is_connect()` and would otherwise be retried on the strict
                // client (pointless — the cert is still broken).
                Ok(Err(e))
                    if !use_relaxed && self.relaxed_client.is_some() && is_cert_error(&e) =>
                {
                    tracing::warn!(
                        "TLS verification failed for {url} ({e}); retrying once with relaxed TLS (tls_unverified)"
                    );
                    use_relaxed = true;
                }
                Ok(Err(e)) if attempt < HTTP_MAX_RETRIES && is_retriable_error(&e) => {
                    tracing::debug!(
                        "transient HTTP error to {url} ({e}), retrying (attempt {})",
                        attempt + 1
                    );
                    attempt += 1;
                    let backoff = HTTP_RETRY_BACKOFF.min(deadline.remaining());
                    if !backoff.is_zero() {
                        tokio::time::sleep(backoff).await;
                    }
                }
                Ok(Err(e)) => {
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

        let final_url_str = resp.url().as_str().to_string();

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

        let final_url = if final_url_str != url {
            Some(final_url_str)
        } else {
            None
        };

        Ok(FetchResult {
            url: url.to_string(),
            final_url,
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
            truncated: false,
            deadline_exceeded: false,
            captured_responses: Vec::new(),
            // HTTP-only path never renders or captures a screenshot.
            screenshot: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_proxy_is_fail_closed_on_bad_url() {
        // A malformed proxy is a hard error — never a silent direct client.
        assert!(
            HttpFetcher::with_proxy("ua", "", false, std::time::Duration::from_secs(5)).is_err()
        );
        assert!(
            HttpFetcher::with_proxy("ua", "not a url", false, std::time::Duration::from_secs(5))
                .is_err()
        );
    }

    #[test]
    fn with_proxy_accepts_valid_url() {
        assert!(
            HttpFetcher::with_proxy(
                "ua",
                "http://user:pass@host:8080",
                false,
                std::time::Duration::from_secs(5),
            )
            .is_ok()
        );
    }
}
