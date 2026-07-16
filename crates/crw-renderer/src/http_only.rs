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
/// TCP connect timeout for the renderer's HTTP tier. A healthy handshake is one
/// RTT (well under a second even intercontinental); by SYN-retransmit timing a
/// connect past ~2.5s means at least two dropped SYNs, i.e. a dead, blocked, or
/// blackholing host whose content is unreliable anyway.
///
/// Lowered 5s -> 2.5s so that inside the SaaS's tight 5s scrape deadline a
/// blackhole surfaces as a connect error with budget left for the fallback-proxy
/// retry (a different egress that CAN reach the origin). At 5s the outer deadline
/// always fired first and the proxy never got a turn.
///
/// This is a single global const shared by the renderer's HTTP tier across scrape,
/// crawl and map (they share one `FallbackRenderer.http`; the SearXNG client has
/// its own separate 5s connect timeout). Crawl/map run on far larger budgets and
/// do not need the lower value, but they are unharmed: a >2.5s connect is a
/// pathological origin, and crawl already skips an unreachable page gracefully. A
/// per-caller connect timeout would need threading through the shared fetcher and
/// is deliberately out of scope here.
const HTTP_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(2500);
/// Overall request timeout for HTTP requests.
const HTTP_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// One retry on transient errors. GET is idempotent so a single retry is safe;
/// origins frequently emit 502/503/504 under brief overload and connect/timeout
/// errors are often DNS or TCP races that resolve on the next attempt.
const HTTP_MAX_RETRIES: u32 = 1;
/// Backoff before the retry attempt. Short — we are inside the request path
/// and the upstream timeout is 30s, so we cannot afford long sleeps.
const HTTP_RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_millis(250);

/// Returns true if a `reqwest::Error` is worth retrying on the SAME egress.
/// Read-phase timeouts only (`is_timeout` without `is_connect`): the origin
/// connected and is slow, so a retry may catch a faster response. Connection-level
/// failures (refused/reset/connect-timeout) are NOT retriable here — they route to
/// the proxy fallback (`is_connection_failure`), or, when no proxy is armed, get a
/// single direct retry at the call site.
fn is_retriable_error(e: &reqwest::Error) -> bool {
    // A read-phase timeout (`is_timeout` without `is_connect`) means we DID connect
    // and the origin is slow to respond — a direct retry may help. Connection-level
    // failures are handled by the proxy arm instead (see `is_connection_failure`),
    // so they are excluded here: retrying them on the same dead egress just wastes
    // budget.
    e.is_timeout() && !e.is_connect()
}

/// True when the direct egress could not talk to the origin at all: the peer
/// refused/reset/aborted the transport, OR the TCP connect timed out (a blackhole
/// that silently drops our SYN). All are signals our egress IP is unwelcome, which
/// a different egress (the fallback proxy) often clears.
///
/// Two distinct reqwest shapes, both verified empirically against this workspace:
///   refused        -> is_connect=true,  chain io::ConnectionRefused
///   reset (RST)    -> is_connect=false, chain io::ConnectionReset  (post-handshake,
///                     hyper_util ErrorKind::SendRequest — this is why `is_connect()`
///                     alone is not enough)
///   connect-timeout-> is_connect=true  AND is_timeout=true, chain io::TimedOut
///   read-timeout   -> is_connect=false, is_timeout=true, no io chain  (EXCLUDED: the
///                     origin answered our SYN and is merely slow; a proxy won't help)
///   dns failure    -> is_connect=true,  no io in chain  (EXCLUDED: the proxy resolves
///                     the same name, so a retry through it is wasted)
fn is_connection_failure(e: &reqwest::Error) -> bool {
    // Connect-phase timeout (blackhole): distinct from a read timeout by is_connect.
    if e.is_connect() && e.is_timeout() {
        return true;
    }
    // Refused / reset / aborted, matched on the transport ErrorKind in the chain.
    let mut src: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(e);
    while let Some(s) = src {
        if let Some(io) = s.downcast_ref::<std::io::Error>()
            && matches!(
                io.kind(),
                std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionAborted
            )
        {
            return true;
        }
        src = std::error::Error::source(s);
    }
    false
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

/// Should the fetch retry once through the fallback proxy? True on an explicit
/// rate-limit status (429) OR when the `cf-mitigated` response header flags a
/// Cloudflare challenge/block — the header is a positive signal, so a different
/// egress IP may clear it even when served as 200/403/503. Pure for unit test.
fn should_arm_proxy(status: u16, cf_mitigated: bool) -> bool {
    is_ratelimit_status(status) || cf_mitigated
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

        // Egress memory: if this host recently hard-blocked our direct egress,
        // start on the proxy instead of re-discovering the block. Without this
        // every URL on a blocking host repeats the whole climb
        // (direct → 429 → proxy retry → JS ladder), which is the 10-20s/URL that
        // made /map time out on sites like Hacker News.
        //
        // We only PREFER the proxy — we never forbid direct (see the rescue arm
        // below). A falsely-latched host whose proxy egress is worse must still be
        // able to succeed, or scrape success would regress.
        let host = url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned));
        // The latch is inert unless a proxy exists AND the deadline can afford both
        // a real proxy attempt and a full direct rescue. Splitting a short budget
        // (the SaaS scrape deadline is 5s) would let a hanging proxy starve the
        // direct rescue, failing a request that direct alone would have served —
        // a scrape-success regression. Below the threshold we behave exactly as
        // before the latch existed.
        let proxy_first = match (&host, self.ratelimit_proxy_client.is_some()) {
            (Some(h), true) => {
                deadline.remaining() >= crate::egress::MIN_BUDGET_FOR_LATCH
                    && crate::egress::global().should_proxy(h).await
            }
            _ => false,
        };
        let mut use_proxy = proxy_first;
        let mut direct_rescue_used = false;
        if proxy_first {
            let m = crw_core::metrics::metrics();
            m.egress_latch_hit_total.inc();
            // Refresh the gauge on the hot latched path so it tracks the live
            // latched-host count (and its TTL decay), mirroring how
            // `host_preferences_size` is set opportunistically during a fetch.
            m.egress_latched_hosts
                .set(crate::egress::global().latched_hosts() as i64);
        }

        let resp = loop {
            let remaining = deadline.remaining();
            if remaining.is_zero() {
                // Already past the budget — report elapsed-since-call so the
                // message reads "Timeout after Xms" instead of a useless 0.
                return Err(CrwError::Timeout(
                    (start.elapsed().as_millis().max(1)) as u64,
                ));
            }
            // While trying the proxy first, hold back a FULL direct-rescue budget. A
            // HANGING proxy is the dangerous case: without the reserve it would eat
            // the whole deadline and direct would never run — the very suppression
            // this design exists to avoid.
            //
            // The reserve is flat, not a share of what's left: a share would shrink
            // the rescue on short deadlines to the point where a healthy direct
            // origin no longer fits in it. `proxy_first` already guarantees the
            // budget is at least MIN_BUDGET_FOR_LATCH, so this subtraction always
            // leaves a real proxy attempt behind.
            let attempt_budget = if use_proxy && proxy_first && !direct_rescue_used {
                remaining.saturating_sub(crate::egress::DIRECT_FALLBACK_RESERVE)
            } else {
                remaining
            };
            if attempt_budget.is_zero() {
                // Not enough left to try the proxy AND still rescue with direct.
                // Spend what remains on direct, which is the attempt we know how to
                // reason about.
                if use_proxy && proxy_first && !direct_rescue_used {
                    use_proxy = false;
                    direct_rescue_used = true;
                    continue;
                }
                return Err(CrwError::Timeout(
                    (start.elapsed().as_millis().max(1)) as u64,
                ));
            }
            let remaining = attempt_budget;
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
                // The proxy-first attempt HUNG until its capped budget ran out.
                // This is the case the reserve exists for: fall back to direct with
                // the budget we held back, instead of failing the whole fetch on a
                // proxy that a latch — possibly a false one — put in front of it.
                Err(_) if use_proxy && proxy_first && !direct_rescue_used => {
                    tracing::warn!(
                        "proxy attempt for {url} exhausted its budget while latched; falling back to direct"
                    );
                    use_proxy = false;
                    direct_rescue_used = true;
                }
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
                        // `direct_rescue_used` means we already tried the proxy for
                        // this request and it failed, which is why we are on direct
                        // at all. Bouncing back to that same broken proxy would just
                        // burn the rescue budget on an attempt we know fails.
                        && !direct_rescue_used
                        && self.ratelimit_proxy_client.is_some()
                        && should_arm_proxy(
                            r.status().as_u16(),
                            r.headers()
                                .get("cf-mitigated")
                                .and_then(|v| v.to_str().ok())
                                .map(crate::detector::is_cloudflare_mitigated_header)
                                .unwrap_or(false),
                        ) =>
                {
                    tracing::warn!(
                        "HTTP {} from {url} (origin rate-limited or cf-mitigated); retrying once via proxy (ratelimit_bypassed)",
                        r.status()
                    );
                    drop(r);
                    // WRITE HOOK — direct-only by construction: this arm is gated on
                    // `!use_proxy`, so the block we just saw was observed on a
                    // genuine direct attempt. Remember it, so the next URL on this
                    // host starts on the proxy instead of paying the climb again.
                    //
                    // A block seen *through* a proxy must never land here: it would
                    // say the proxy is blocked, not direct, and would let one
                    // caller's broken proxy demote every other caller's healthy
                    // direct traffic onto paid bandwidth.
                    if let Some(h) = &host {
                        let eg = crate::egress::global();
                        eg.note_block(h).await;
                        crw_core::metrics::metrics()
                            .egress_latched_hosts
                            .set(eg.latched_hosts() as i64);
                    }
                    use_proxy = true;
                }
                // The proxy-first attempt did not return a usable page. Direct is
                // not forbidden by a latch — only deprioritized — so spend the
                // reserved budget on one direct attempt rather than returning the
                // proxy's result. This keeps best-result behaviour when the proxy
                // is the worse egress for this host (origin blocks the proxy
                // ranges, bad geo exit, a 403/5xx wall the box's own IP clears).
                //
                // Gate on "not a usable page", which is BROADER than the 429/cf
                // signal the direct-side arm uses: the latch reorders and must
                // never SUPPRESS direct, so ANY non-2xx proxy response (403, 5xx,
                // an un-followed redirect) has to leave a direct rescue reachable,
                // or a falsely-latched host would return the proxy's failure while
                // direct would have served 200 — a scrape-success regression.
                //
                // `cf-mitigated` is folded in explicitly because a Cloudflare
                // challenge is often served as a 200: `is_success()` alone would
                // let that interstitial through and skip the rescue, silently
                // narrowing the very case (`should_arm_proxy(200, true)`) the
                // pre-existing 429 arm already treats as a block. Retriable
                // statuses still exhaust their retries on the proxy first (that arm
                // is matched above); a clean 2xx falls through to `break r` and
                // never wastes the rescue.
                Ok(Ok(r))
                    if use_proxy
                        && proxy_first
                        && !direct_rescue_used
                        && (!r.status().is_success()
                            || r.headers()
                                .get("cf-mitigated")
                                .and_then(|v| v.to_str().ok())
                                .map(crate::detector::is_cloudflare_mitigated_header)
                                .unwrap_or(false)) =>
                {
                    tracing::warn!(
                        "HTTP {} from {url} via proxy while latched; falling back to direct",
                        r.status()
                    );
                    drop(r);
                    use_proxy = false;
                    direct_rescue_used = true;
                }
                Ok(Ok(r)) => break r,
                // TLS cert verification failed and relaxed-TLS fallback is armed:
                // swap to the cert-disabled client and retry once. NOT a transient
                // retry — does not consume the retry budget or back off. Placed
                // before the generic retry arm because cert failures are
                // `is_connect()` and would otherwise be retried on the strict
                // client (pointless — the cert is still broken).
                // The proxy-first attempt could not reach the origin AT ALL (proxy
                // down, refused, DNS, misconfigured creds). The latch only expresses
                // a preference, so this must not sink the fetch: spend the reserved
                // budget on direct. Without this arm an unreachable proxy would turn
                // every scrape of a latched host into a hard failure for the whole
                // cooldown — a scrape-success regression, and precisely the case the
                // "reorder, never suppress" rule exists to prevent.
                Ok(Err(_)) if use_proxy && proxy_first && !direct_rescue_used => {
                    tracing::warn!(
                        "proxy egress failed for {url} while latched; falling back to direct"
                    );
                    use_proxy = false;
                    direct_rescue_used = true;
                }
                Ok(Err(e))
                    if !use_relaxed && self.relaxed_client.is_some() && is_cert_error(&e) =>
                {
                    tracing::warn!(
                        "TLS verification failed for {url} ({e}); retrying once with relaxed TLS (tls_unverified)"
                    );
                    use_relaxed = true;
                }
                // The direct egress could not reach the origin (refused / reset /
                // aborted / connect-timeout blackhole). A direct retry would just hit
                // the same dead path, so switch straight to the fallback proxy — a
                // different egress that often CAN reach the origin. This is the fix for
                // the production case: prod's IPv4 is blocked/blackholed by many origins
                // that the DataImpulse pool reaches in ~1.6s.
                //
                // Placed BEFORE the generic retriable arm so a connection failure spends
                // its one alternate attempt on the proxy rather than wasting the tight
                // 5s budget on a doomed direct retry. `!use_proxy` bounds it to a single
                // switch (worst case: direct, proxy, proxy-retry). Arming does not
                // consume the transient retry budget, matching the 429 arm.
                //
                // Skipped when too little budget remains for a proxy round trip to
                // plausibly complete — this is exactly why HTTP_CONNECT_TIMEOUT was
                // lowered: a blackhole must fail early enough to leave that budget.
                //
                // Note `active_client` prefers the proxy over the relaxed-TLS client, so
                // a site needing BOTH a relaxed cert and a different egress keeps the
                // strict-TLS proxy client and still fails; the 429 arm has the same
                // property and no observed origin needs both.
                Ok(Err(e))
                    if !use_proxy
                        && self.ratelimit_proxy_client.is_some()
                        && deadline.remaining() >= crate::MIN_TIER_BUDGET
                        && is_connection_failure(&e) =>
                {
                    tracing::warn!(
                        "direct egress could not reach {url} ({e}); retrying via proxy (egress_blocked)"
                    );
                    use_proxy = true;
                }
                // Retry once on the same egress. Read-phase timeouts qualify (the origin
                // connected and is slow — a retry may help). Connection-level failures
                // qualify ONLY when no proxy is armed: with a proxy, the arm above already
                // handled them; without one (the OSS/self-host default, where the
                // DataImpulse fallback is unset), this preserves the pre-existing safety
                // net for a transient refused/reset/DNS blip that a retry often clears.
                Ok(Err(e))
                    if attempt < HTTP_MAX_RETRIES
                        && (is_retriable_error(&e)
                            || (self.ratelimit_proxy_client.is_none()
                                && is_connection_failure(&e))) =>
                {
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
                    // A connect-phase failure to the ORIGIN (refused / DNS / TLS-handshake /
                    // connect-timeout) means the caller's target is unreachable → 422. But
                    // once we switched to the proxy, a failure may be the proxy infra's
                    // fault, not the origin's, so keep it a 502 (our side) rather than
                    // blaming the caller. A post-handshake reset likewise stays 502.
                    return Err(if e.is_connect() && !use_proxy {
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

        let content_type_header = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let content_type = content_type_header
            .as_deref()
            .map(|s| s.split(';').next().unwrap_or(s).trim().to_lowercase());
        // Charset from the Content-Type header (P1-1): pages served as Latin-1 /
        // Windows-1252 would otherwise be UTF-8-lossy'd, turning each 0x80–0xFF
        // byte into U+FFFD. Kept separately since `content_type` drops it.
        let header_charset = content_type_header
            .as_deref()
            .and_then(charset_from_content_type);

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
            (decode_html_bytes(&bytes, header_charset.as_deref()), None)
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

/// Extract the `charset` label from a `Content-Type` header value
/// (e.g. `text/html; charset=ISO-8859-1` → `ISO-8859-1`).
fn charset_from_content_type(ct: &str) -> Option<String> {
    let lower = ct.to_ascii_lowercase();
    let idx = lower.find("charset")?;
    let after = ct[idx + "charset".len()..].trim_start();
    let after = after.strip_prefix('=')?.trim_start();
    let after = after.trim_start_matches(['"', '\'']);
    let end = after
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':'))
        .unwrap_or(after.len());
    let label = after[..end].trim();
    (!label.is_empty()).then(|| label.to_string())
}

/// Sniff a `<meta charset>` / `<meta http-equiv=content-type … charset=…>`
/// declaration from the first ~2KB of an HTML document.
fn sniff_meta_charset(bytes: &[u8]) -> Option<String> {
    let head = &bytes[..bytes.len().min(2048)];
    let text = String::from_utf8_lossy(head).to_ascii_lowercase();
    let idx = text.find("charset")?;
    let after = text[idx + "charset".len()..].trim_start();
    let after = after.strip_prefix('=')?.trim_start();
    let after = after.trim_start_matches(['"', '\'']);
    let end = after
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .unwrap_or(after.len());
    let label = &after[..end];
    (!label.is_empty()).then(|| label.to_string())
}

/// Decode fetched HTML bytes to a `String` honoring the declared charset
/// (P1-1): HTTP `Content-Type` charset first, then a `<meta charset>` sniff,
/// then UTF-8. Without this, a Latin-1 / Windows-1252 page has every 0x80–0xFF
/// byte replaced with U+FFFD.
fn decode_html_bytes(bytes: &[u8], header_charset: Option<&str>) -> String {
    // Header charset wins, but a bogus/unknown header label must still fall
    // through to a <meta charset> sniff before giving up on UTF-8.
    let enc = header_charset
        .and_then(|l| encoding_rs::Encoding::for_label(l.as_bytes()))
        .or_else(|| {
            sniff_meta_charset(bytes).and_then(|l| encoding_rs::Encoding::for_label(l.as_bytes()))
        });
    match enc {
        Some(enc) => enc.decode(bytes).0.into_owned(),
        None => String::from_utf8_lossy(bytes).into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_arm_proxy_truth_table() {
        assert!(should_arm_proxy(429, false), "429 arms on its own");
        assert!(
            !should_arm_proxy(403, false),
            "403 without header does not arm"
        );
        assert!(should_arm_proxy(403, true), "403 + cf-mitigated arms");
        assert!(should_arm_proxy(200, true), "challenge served as 200 arms");
        assert!(!should_arm_proxy(200, false), "clean 200 does not arm");
    }

    /// Complete the handshake, read the request, THEN abort with RST
    /// (unix only: forcing an RST needs SO_LINGER; on other platforms `close()`
    /// sends FIN and the test would assert the wrong error class).
    /// This is the WAF-style block seen in production: the
    /// origin inspects the request before rejecting it. Reading first is what makes
    /// the test deterministic — it forces the error into hyper's send-request phase
    /// rather than the connect phase.
    #[cfg(unix)]
    fn spawn_resetting_origin() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                use std::io::Read;
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                #[cfg(unix)]
                unsafe {
                    use std::os::fd::AsRawFd;
                    let l = libc::linger {
                        l_onoff: 1,
                        l_linger: 0,
                    };
                    libc::setsockopt(
                        stream.as_raw_fd(),
                        libc::SOL_SOCKET,
                        libc::SO_LINGER,
                        std::ptr::from_ref(&l).cast(),
                        std::mem::size_of::<libc::linger>() as libc::socklen_t,
                    );
                }
                drop(stream);
            }
        });
        format!("http://{addr}/")
    }

    /// A reset on an established connection must arm the proxy. `is_connect()` is
    /// false here — which is exactly why the predicate cannot be built on it. This
    /// mirrors the production trace for `sabir.com`, where the engine reported
    /// `HttpError` ("HTTP request failed") rather than `TargetUnreachable`, proving
    /// `is_connect()` was false for the real block.
    #[cfg(unix)]
    #[tokio::test]
    async fn connection_failure_catches_connection_reset() {
        let url = spawn_resetting_origin();
        let err = reqwest::Client::new().get(&url).send().await.unwrap_err();
        assert!(
            !err.is_connect(),
            "a post-handshake reset must not be is_connect(); \
             if this ever flips, the arm ordering below needs revisiting"
        );
        assert!(is_connection_failure(&err), "reset must arm the proxy");
    }

    /// A refused connection (nothing listening) must also arm the proxy.
    #[tokio::test]
    async fn connection_failure_catches_connection_refused() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let err = reqwest::Client::new()
            .get(format!("http://{addr}/"))
            .send()
            .await
            .unwrap_err();
        assert!(is_connection_failure(&err), "refused must arm the proxy");
    }

    /// A connect-phase TIMEOUT (blackhole that drops our SYN) must arm the proxy —
    /// this is the dominant production block. It is distinguished from a read timeout
    /// by `is_connect() && is_timeout()`. 192.0.2.1 is RFC 5737 TEST-NET-1, guaranteed
    /// never routed, so the SYN is blackholed and the connect times out.
    #[tokio::test]
    async fn connection_failure_catches_connect_timeout() {
        let err = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_millis(300))
            .build()
            .unwrap()
            .get("http://192.0.2.1/")
            .send()
            .await
            .unwrap_err();
        assert!(
            err.is_connect() && err.is_timeout(),
            "guard the assumption: a blackhole is a connect-phase timeout (got {err:?})"
        );
        assert!(
            is_connection_failure(&err),
            "connect-timeout blackhole must arm the proxy"
        );
    }

    /// A READ timeout (origin accepted the connection, then stalled) must NOT arm the
    /// proxy — a different egress cannot make a slow origin faster.
    #[tokio::test]
    async fn connection_failure_ignores_read_timeout() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                std::thread::sleep(std::time::Duration::from_secs(30));
                drop(stream);
            }
        });
        let err = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(400))
            .build()
            .unwrap()
            .get(format!("http://{addr}/"))
            .send()
            .await
            .unwrap_err();
        assert!(
            err.is_timeout() && !err.is_connect(),
            "guard the assumption: a post-connect stall is a read timeout (got {err:?})"
        );
        assert!(
            !is_connection_failure(&err),
            "a read timeout must not arm the proxy"
        );
    }

    /// Minimal forward proxy: reads the absolute-URI request reqwest sends for a
    /// plain-HTTP proxied GET and answers 200 with a marker body.
    #[cfg(unix)]
    fn spawn_stub_proxy() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                use std::io::{Read, Write};
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf);
                const BODY: &str = "<html><body>served via proxy</body></html>";
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{BODY}",
                        BODY.len()
                    )
                    .as_bytes(),
                );
            }
        });
        format!("http://{addr}")
    }

    /// End-to-end: an origin that resets the connection is retried through the
    /// fallback proxy and succeeds. This is the production `sabir.com` case
    /// (prod egress IP refused, proxy reaches it in ~1.6s). Without the
    /// `is_connection_failure` arm the request dies with `HttpError` and the caller
    /// sees a 502.
    #[cfg(unix)]
    #[tokio::test]
    async fn connection_reset_is_retried_through_proxy() {
        let origin = spawn_resetting_origin();
        let proxy = spawn_stub_proxy();

        let fetcher = HttpFetcher {
            client: reqwest::Client::new(),
            relaxed_client: None,
            ratelimit_proxy_client: Some(
                build_client(
                    "test-ua",
                    Some(&proxy),
                    std::time::Duration::from_secs(5),
                    false,
                )
                .unwrap(),
            ),
            inject_stealth_headers: false,
        };

        let res = fetcher
            .fetch(
                &origin,
                &HashMap::new(),
                None,
                Deadline::from_request_ms(10_000),
            )
            .await
            .expect("reset origin must be recovered through the proxy");
        assert!(
            res.html.contains("served via proxy"),
            "expected the proxy's body, got: {}",
            res.html
        );
    }

    /// Without an armed proxy the same reset still fails — the arm is what fixes
    /// it, not some incidental retry.
    #[cfg(unix)]
    #[tokio::test]
    async fn connection_reset_without_proxy_still_fails() {
        let origin = spawn_resetting_origin();
        let fetcher = HttpFetcher {
            client: reqwest::Client::new(),
            relaxed_client: None,
            ratelimit_proxy_client: None,
            inject_stealth_headers: false,
        };
        let err = fetcher
            .fetch(
                &origin,
                &HashMap::new(),
                None,
                Deadline::from_request_ms(10_000),
            )
            .await
            .expect_err("no proxy armed => the reset must surface as an error");
        assert!(
            matches!(err, CrwError::HttpError(_) | CrwError::TargetUnreachable(_)),
            "unexpected error variant: {err:?}"
        );
    }

    /// Like `spawn_resetting_origin` but counts how many connections it served, so a
    /// test can prove a connection failure was retried.
    #[cfg(unix)]
    fn spawn_resetting_counter() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c = counter.clone();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                use std::io::Read;
                c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                unsafe {
                    use std::os::fd::AsRawFd;
                    let l = libc::linger {
                        l_onoff: 1,
                        l_linger: 0,
                    };
                    libc::setsockopt(
                        stream.as_raw_fd(),
                        libc::SOL_SOCKET,
                        libc::SO_LINGER,
                        std::ptr::from_ref(&l).cast(),
                        std::mem::size_of::<libc::linger>() as libc::socklen_t,
                    );
                }
                drop(stream);
            }
        });
        (format!("http://{addr}/"), counter)
    }

    /// No proxy armed: a connection failure must still get one direct retry, so the
    /// self-host path keeps its pre-existing resilience to a transient blip.
    #[cfg(unix)]
    #[tokio::test]
    async fn connection_failure_without_proxy_retries_once() {
        let (origin, hits) = spawn_resetting_counter();
        let fetcher = HttpFetcher {
            client: reqwest::Client::new(),
            relaxed_client: None,
            ratelimit_proxy_client: None,
            inject_stealth_headers: false,
        };
        let _ = fetcher
            .fetch(
                &origin,
                &HashMap::new(),
                None,
                Deadline::from_request_ms(10_000),
            )
            .await;
        // Initial attempt + one retry = 2 connects. Without the no-proxy retry branch
        // it would be 1.
        assert_eq!(
            hits.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "a connection failure with no proxy must be retried once directly"
        );
    }

    /// When the proxy is tried and it ALSO fails to connect, the error must stay a
    /// 502-class HttpError, not a 422 TargetUnreachable: a proxy connect failure can
    /// be our infra, not proof the caller's target is dead.
    #[tokio::test]
    async fn proxy_connect_failure_is_not_blamed_on_the_caller() {
        // origin: refused. proxy: also refused (closed port).
        let o = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let oaddr = o.local_addr().unwrap();
        drop(o);
        let p = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let paddr = p.local_addr().unwrap();
        drop(p);
        let fetcher = HttpFetcher {
            client: reqwest::Client::new(),
            relaxed_client: None,
            ratelimit_proxy_client: Some(
                build_client(
                    "ua",
                    Some(&format!("http://{paddr}")),
                    std::time::Duration::from_secs(5),
                    false,
                )
                .unwrap(),
            ),
            inject_stealth_headers: false,
        };
        let err = fetcher
            .fetch(
                &format!("http://{oaddr}/"),
                &HashMap::new(),
                None,
                Deadline::from_request_ms(10_000),
            )
            .await
            .expect_err("both origin and proxy refuse");
        assert!(
            matches!(err, CrwError::HttpError(_)),
            "a proxy-side failure must not be reported as TargetUnreachable (422); got {err:?}"
        );
    }

    /// A DNS failure must NOT arm the proxy: it resolves the same name, so the extra
    /// round trip buys nothing. Written to be network-tolerant: if the environment's
    /// resolver hijacks NXDOMAIN and returns a response, there is no error to classify
    /// and the assertion is vacuous rather than flaky.
    #[tokio::test]
    async fn connection_failure_ignores_dns_failure() {
        let res = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap()
            .get("http://nonexistent.invalid./")
            .send()
            .await;
        if let Err(err) = res {
            assert!(
                !is_connection_failure(&err),
                "DNS failure must not arm the proxy (got {err:?})"
            );
        }
    }

    #[test]
    fn charset_from_content_type_parses_label() {
        assert_eq!(
            charset_from_content_type("text/html; charset=ISO-8859-1").as_deref(),
            Some("ISO-8859-1")
        );
        assert_eq!(
            charset_from_content_type("text/html;charset=\"utf-8\"").as_deref(),
            Some("utf-8")
        );
        assert_eq!(charset_from_content_type("text/html").as_deref(), None);
    }

    #[test]
    fn decode_latin1_via_header_no_replacement_char() {
        // "café ûber" in ISO-8859-1: é=0xE9, û=0xFB.
        let bytes = b"caf\xE9 \xFBber";
        let out = decode_html_bytes(bytes, Some("iso-8859-1"));
        assert_eq!(out, "café ûber");
        assert!(!out.contains('\u{FFFD}'));
    }

    #[test]
    fn decode_windows1254_via_meta_sniff() {
        // Turkish "için" in Windows-1254: i=0x69, ç=0xE7, i=0x69, n=0x6E.
        let bytes = b"<meta charset=windows-1254><p>i\xE7in</p>";
        let out = decode_html_bytes(bytes, None);
        assert!(out.contains("için"), "got: {out}");
        assert!(!out.contains('\u{FFFD}'));
    }

    #[test]
    fn decode_bogus_header_falls_through_to_meta_then_utf8() {
        // Bogus header label must NOT short-circuit to UTF-8 — a valid <meta>
        // charset should still win. Turkish "için" in Windows-1254.
        let bytes = b"<meta charset=windows-1254><p>i\xE7in</p>";
        let out = decode_html_bytes(bytes, Some("x-bogus-nonsense"));
        assert!(out.contains("için"), "got: {out}");
        // Bogus header + no meta → UTF-8 lossy fallback (no panic).
        let plain = decode_html_bytes(b"plain ascii", Some("x-bogus"));
        assert_eq!(plain, "plain ascii");
    }

    #[test]
    fn decode_utf8_unchanged() {
        let bytes = "café İstanbul 東京".as_bytes();
        assert_eq!(
            decode_html_bytes(bytes, Some("utf-8")),
            "café İstanbul 東京"
        );
        // No charset info → still UTF-8 by default.
        assert_eq!(decode_html_bytes(bytes, None), "café İstanbul 東京");
    }

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
