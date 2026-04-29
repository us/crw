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
//! let result = renderer.fetch("https://example.com", &HashMap::new(), None, None, None).await?;
//! println!("status: {}", result.status_code);
//! # Ok(())
//! # }
//! ```

pub mod breaker;
#[cfg(feature = "auto-browser")]
pub mod browser;
#[cfg(feature = "cdp")]
pub mod cdp;
#[cfg(feature = "cdp")]
pub mod cdp_conn;
pub mod detector;
pub mod http_only;
pub mod preference;
pub mod traits;

use crate::breaker::{BreakerRegistry, Permit, ProbeGuard};
use crate::preference::HostPreferences;
use crw_core::config::{BUILTIN_UA_POOL, RendererConfig, RendererMode, StealthConfig};
use crw_core::error::{CrwError, CrwResult};
use crw_core::metrics::metrics;
use crw_core::types::{
    FailoverErrorKind, FetchResult, RenderDecision, RendererKind, resolve_render_js,
};
use std::collections::HashMap;
use std::sync::Arc;
use traits::PageFetcher;

/// Map a renderer's name string to the closed `RendererKind` enum.
/// Returns `None` for unknown names (e.g. "playwright" — treated as a
/// JS renderer but not tracked in metrics/preferences).
fn renderer_kind_for(name: &str) -> Option<RendererKind> {
    match name {
        "http" | "http_only_fallback" => Some(RendererKind::Http),
        "lightpanda" => Some(RendererKind::Lightpanda),
        "chrome" => Some(RendererKind::Chrome),
        _ => None,
    }
}

/// Classify a renderer-side error into a `FailoverErrorKind` for the
/// preference learner. Match on `CrwError` variants (not error strings),
/// so renaming or rewording the human-readable message can't silently
/// reclassify failures and over-promote hosts.
///
/// Only LightPanda-specific failures drive promotion (see
/// [`FailoverErrorKind::counts_for_promotion`]); transport / unreachable
/// errors stay in `NetworkError` so a flaky upstream doesn't push hosts
/// to Chrome.
fn classify_renderer_error(err: &CrwError) -> FailoverErrorKind {
    match err {
        CrwError::Timeout(_) => FailoverErrorKind::LightpandaTimeout,
        CrwError::TargetUnreachable(_) => FailoverErrorKind::NetworkError,
        CrwError::HttpError(_) => FailoverErrorKind::NetworkError,
        // RendererError covers WS disconnects, CDP frame errors, render
        // pipeline crashes — these are LightPanda-attributable.
        CrwError::RendererError(_) => FailoverErrorKind::LightpandaCrash,
        _ => FailoverErrorKind::Other,
    }
}

/// Per-renderer credit cost. Exposed so the routing layer can populate
/// `FetchResult.credit_cost` and `/v1/scrape` charge accurately.
fn credit_for(kind: RendererKind) -> u32 {
    match kind {
        RendererKind::Http => 1,
        RendererKind::Lightpanda => 1,
        RendererKind::Chrome => 2,
    }
}

/// Stamp `render_decision` and `credit_cost` for an HTTP-only result.
/// `requested_renderer` is taken into account: if the user explicitly
/// pinned `"http"` we mark it as `UserPinned`, otherwise `AutoDefault`.
fn stamp_http_decision(result: &mut FetchResult, requested_renderer: Option<&str>) {
    if result.render_decision.is_some() {
        return;
    }
    let kind = RendererKind::Http;
    result.credit_cost = credit_for(kind);
    result.render_decision = Some(match requested_renderer {
        Some("http") => RenderDecision::UserPinned { renderer: kind },
        _ => RenderDecision::AutoDefault { chosen: kind },
    });
    // Mirror the JS-renderer metric so dashboards see HTTP routing too.
    metrics()
        .render_route_decision_total
        .with_label_values(&[kind.as_str(), "success"])
        .inc();
}

/// Extract the host from a URL string, returning an empty string on failure.
fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_default()
}

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
    /// Per-host renderer preference learning (auto-mode only).
    preferences: Arc<HostPreferences>,
    /// Per-host + global circuit breakers per renderer.
    breakers: Arc<BreakerRegistry>,
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
                preferences: Arc::new(HostPreferences::with_defaults()),
                breakers: Arc::new(BreakerRegistry::with_defaults()),
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
            preferences: Arc::new(HostPreferences::with_defaults()),
            breakers: Arc::new(BreakerRegistry::with_defaults()),
        })
    }

    /// Access the host preferences cache (for admin endpoints, tests).
    pub fn preferences(&self) -> Arc<HostPreferences> {
        Arc::clone(&self.preferences)
    }

    /// Access the breaker registry (for tests).
    pub fn breakers(&self) -> Arc<BreakerRegistry> {
        Arc::clone(&self.breakers)
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
        requested_renderer: Option<&str>,
    ) -> CrwResult<FetchResult> {
        let effective = resolve_render_js(render_js, self.render_js_default);
        tracing::debug!(
            url,
            request_render_js = ?render_js,
            default_render_js = ?self.render_js_default,
            effective_render_js = ?effective,
            requested_renderer,
            "FallbackRenderer::fetch dispatching"
        );
        // A non-"auto" pinned renderer is a hard pin — failures must surface.
        let is_hard_pinned = matches!(requested_renderer, Some(name) if name != "auto");
        match effective {
            Some(false) => {
                let mut r = self.http.fetch(url, headers, None).await?;
                stamp_http_decision(&mut r, requested_renderer);
                Ok(r)
            }
            Some(true) => {
                // Fetch via HTTP first to check content type — PDFs can't be JS-rendered.
                let mut http_result = self.http.fetch(url, headers, None).await?;
                if http_result.content_type.as_deref() == Some("application/pdf") {
                    stamp_http_decision(&mut http_result, requested_renderer);
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
                    result.warnings.push(
                        "JS rendering requested but no renderer available; HTTP fallback used"
                            .into(),
                    );
                    stamp_http_decision(&mut result, requested_renderer);
                    Ok(result)
                } else {
                    self.fetch_with_js(url, headers, wait_for_ms, requested_renderer)
                        .await
                }
            }
            None => {
                // In auto mode, an HTTP-layer failure (TargetUnreachable, body
                // decode mid-stream, oversize response, transient network) is
                // not terminal: if a JS renderer is available, escalate. Many
                // sites that reject reqwest's TLS/UA fingerprint succeed via a
                // real Chromium navigation. Bench analysis: 10/147 false
                // "unreachable" + 5/147 "http_502" map to this branch.
                let mut result = match self.http.fetch(url, headers, None).await {
                    Ok(r) => r,
                    Err(e) if !self.js_renderers.is_empty() => {
                        tracing::info!(
                            url,
                            error = %e,
                            "HTTP fetch failed, escalating to JS renderer"
                        );
                        return self
                            .fetch_with_js(url, headers, wait_for_ms, requested_renderer)
                            .await
                            .map_err(|js_err| {
                                tracing::warn!("Both HTTP and JS failed: http={e}, js={js_err}");
                                js_err
                            });
                    }
                    Err(e) => return Err(e),
                };

                // PDFs don't need JS rendering — return immediately.
                if result.content_type.as_deref() == Some("application/pdf") {
                    stamp_http_decision(&mut result, requested_renderer);
                    return Ok(result);
                }

                let needs_js = detector::needs_js_rendering(&result.html);
                let cf_header_signal = result.warning.as_deref() == Some("cloudflare_mitigated");
                let is_blocked =
                    cf_header_signal || detector::looks_like_cloudflare_challenge(&result.html);
                let is_auth_blocked = matches!(result.status_code, 401 | 403);
                // Post-fetch thin-content trigger: HTTP returned 2xx but the
                // body has effectively no extractable text. Catches sites whose
                // SPA marker we don't recognize (no `id="root"`, no
                // `__next_data__`) yet still return a near-empty HTML shell.
                // Bench analysis showed 23/147 failures fall in this bucket
                // (seattletimes, espn, ionos, huduser, …).
                let is_2xx = (200..300).contains(&result.status_code);
                let is_thin_content = is_2xx && detector::looks_like_thin_html(&result.html);

                if !self.js_renderers.is_empty()
                    && (needs_js || is_blocked || is_auth_blocked || is_thin_content)
                {
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
                    } else if needs_js {
                        tracing::info!(url, "SPA shell detected, retrying with JS renderer");
                    } else {
                        tracing::info!(
                            url,
                            html_len = result.html.len(),
                            "HTTP 2xx but body is thin, escalating to JS renderer"
                        );
                    }
                    match self
                        .fetch_with_js(url, headers, wait_for_ms, requested_renderer)
                        .await
                    {
                        Ok(js_result) => Ok(js_result),
                        Err(e) if is_hard_pinned => {
                            // User explicitly pinned a renderer — surface the error
                            // instead of silently returning the (likely useless) HTTP body.
                            Err(e)
                        }
                        Err(e) => {
                            tracing::warn!("JS rendering failed, falling back to HTTP result: {e}");
                            stamp_http_decision(&mut result, requested_renderer);
                            Ok(result)
                        }
                    }
                } else {
                    stamp_http_decision(&mut result, requested_renderer);
                    Ok(result)
                }
            }
        }
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
        requested_renderer: Option<&str>,
    ) -> CrwResult<FetchResult> {
        let host = host_of(url);
        let is_user_pinned = matches!(requested_renderer, Some(name) if name != "auto");
        if let Some(pinned) = requested_renderer
            && let Some(kind) = renderer_kind_for(pinned)
        {
            metrics()
                .user_pin_total
                .with_label_values(&[kind.as_str()])
                .inc();
        }

        // Filter the JS pool down to a hard-pinned renderer when one was named.
        // "auto" or `None` means "use the configured chain".
        let mut renderers: Vec<&Arc<dyn PageFetcher>> = match requested_renderer {
            Some(name) if name != "auto" => self
                .js_renderers
                .iter()
                .filter(|r| r.name() == name)
                .collect(),
            _ => self.js_renderers.iter().collect(),
        };

        // Auto mode: if this host has been promoted, try Chrome first.
        if !is_user_pinned
            && let Some(RendererKind::Chrome) = self.preferences.preferred(&host).await
        {
            renderers.sort_by_key(|r| if r.name() == "chrome" { 0 } else { 1 });
            tracing::debug!(host = %host, "host promoted to chrome by preference learner");
        }

        if renderers.is_empty() {
            let available = self.js_renderer_names();
            return Err(CrwError::RendererError(format!(
                "requested renderer '{}' not in pool [{}]",
                requested_renderer.unwrap_or("auto"),
                available.join(", ")
            )));
        }

        // Track the chain we attempted so we can populate
        // `RenderDecision::Failover` when nothing succeeded outright.
        let mut chain: Vec<RendererKind> = Vec::new();
        let mut breaker_skipped: Vec<RendererKind> = Vec::new();
        let mut last_error = None;
        let mut last_failover_reason: Option<FailoverErrorKind> = None;
        let mut thin_result: Option<FetchResult> = None;

        for renderer in renderers {
            let kind = renderer_kind_for(renderer.name());

            // Skip empty hosts: don't pollute breaker/preference caches
            // with the "" key when URL parsing failed.
            let trackable = kind.filter(|_| !host.is_empty());

            // Consult breaker for tracked renderers. Untracked names (e.g.
            // "playwright") bypass the breaker for now.
            let mut probe_guard: Option<ProbeGuard> = None;
            if let Some(k) = trackable {
                let (permit, guard) = self.breakers.acquire_with_guard(&host, k).await;
                if permit == Permit::Rejected {
                    tracing::info!(
                        renderer = renderer.name(),
                        host = %host,
                        "circuit breaker open, skipping renderer"
                    );
                    metrics()
                        .render_route_decision_total
                        .with_label_values(&[k.as_str(), "breakerSkipped"])
                        .inc();
                    breaker_skipped.push(k);
                    drop(guard); // not Probe — drop is a no-op
                    continue;
                }
                probe_guard = Some(guard);
            }
            if let Some(k) = kind {
                chain.push(k);
            }

            match renderer.fetch(url, headers, wait_for_ms).await {
                Ok(mut result) => {
                    let text_len = html_body_text_len(&result.html);
                    let is_placeholder = detector::looks_like_loading_placeholder(&result.html);
                    let failed_render = detector::looks_like_failed_render(&result.html);
                    if text_len >= Self::MIN_RENDERED_TEXT_LEN
                        && !is_placeholder
                        && failed_render.is_none()
                    {
                        // Capture the promotion state BEFORE record_success
                        // clears the latch — otherwise AutoPromoted decisions
                        // race against the success path and downgrade to AutoDefault.
                        let was_promoted = matches!(
                            self.preferences.preferred(&host).await,
                            Some(RendererKind::Chrome)
                        );
                        if let Some(k) = trackable {
                            self.breakers.record_result(&host, k, true).await;
                            self.preferences.record_success(&host).await;
                            metrics()
                                .render_route_decision_total
                                .with_label_values(&[k.as_str(), "success"])
                                .inc();
                            metrics()
                                .host_preferences_size
                                .set(self.preferences.size() as i64);
                        }
                        if let Some(g) = probe_guard.take() {
                            g.disarm();
                        }
                        // Populate routing metadata + per-renderer credit.
                        if let Some(k) = kind {
                            result.credit_cost = credit_for(k);
                            result.render_decision = Some(if is_user_pinned {
                                RenderDecision::UserPinned { renderer: k }
                            } else if !breaker_skipped.is_empty() {
                                RenderDecision::BreakerSkipped {
                                    skipped: breaker_skipped[0],
                                    chosen: k,
                                }
                            } else if chain.len() > 1 {
                                RenderDecision::Failover {
                                    chain: chain.clone(),
                                    reason: last_failover_reason
                                        .clone()
                                        .unwrap_or(FailoverErrorKind::Other),
                                }
                            } else if was_promoted && k == RendererKind::Chrome {
                                RenderDecision::AutoPromoted {
                                    chosen: k,
                                    from: RendererKind::Lightpanda,
                                    reason: "host preference learner".into(),
                                }
                            } else {
                                RenderDecision::AutoDefault { chosen: k }
                            });
                        }
                        return Ok(result);
                    }
                    // Treat thin/placeholder/failed as a soft failure for
                    // breaker + preference purposes.
                    let err_kind = match failed_render {
                        Some(detector::FailedRenderReason::NextJsClientError) => {
                            FailoverErrorKind::NextJsClientError
                        }
                        Some(detector::FailedRenderReason::ReactMinifiedError) => {
                            FailoverErrorKind::NextJsClientError
                        }
                        Some(detector::FailedRenderReason::EmptyNextRoot) => {
                            FailoverErrorKind::EmptyNextRoot
                        }
                        None if is_placeholder => FailoverErrorKind::PlaceholderContent,
                        None => FailoverErrorKind::PlaceholderContent,
                    };
                    last_failover_reason = Some(err_kind.clone());
                    if let Some(k) = trackable {
                        self.breakers.record_result(&host, k, false).await;
                        if k == RendererKind::Lightpanda
                            && let Some(target) =
                                self.preferences.record_failure(&host, &err_kind).await
                        {
                            metrics()
                                .host_preferences_promotions_total
                                .with_label_values(&[k.as_str(), target.as_str()])
                                .inc();
                            tracing::info!(
                                host = %host,
                                "host promoted by preference learner: {} -> {}",
                                k.as_str(),
                                target.as_str()
                            );
                        }
                    }
                    if let Some(g) = probe_guard.take() {
                        g.disarm();
                    }
                    tracing::info!(
                        renderer = renderer.name(),
                        text_len,
                        is_placeholder,
                        failed_render = ?failed_render,
                        "JS renderer returned thin/placeholder/failed content, trying next renderer"
                    );
                    // Annotate the result so it can surface through `thin_result`
                    // if no later renderer succeeds. Preserves any warning the
                    // renderer set, but adds the failover reason. We keep the
                    // first thin result as the body to return (no point in
                    // accumulating bodies), but stitch later renderers'
                    // warnings onto it so debug output reflects every attempt.
                    let mut annotated = result;
                    let attempt_warning = if let Some(reason) = failed_render {
                        format!(
                            "{} returned a failed render ({})",
                            renderer.name(),
                            reason.as_str()
                        )
                    } else if is_placeholder {
                        format!("{} returned a loading placeholder", renderer.name())
                    } else {
                        format!(
                            "{} returned thin content (text_len={text_len})",
                            renderer.name()
                        )
                    };
                    annotated.warnings.push(attempt_warning.clone());
                    annotated.warning = Some(match annotated.warning {
                        Some(prev) => format!("{prev}; {attempt_warning}"),
                        None => attempt_warning.clone(),
                    });
                    thin_result = Some(match thin_result {
                        None => annotated,
                        Some(mut existing) => {
                            existing.warnings.push(attempt_warning.clone());
                            existing.warning = Some(match existing.warning {
                                Some(prev) => format!("{prev}; {attempt_warning}"),
                                None => attempt_warning,
                            });
                            existing
                        }
                    });
                }
                Err(e) => {
                    tracing::warn!(renderer = renderer.name(), "JS renderer failed: {e}");
                    let err_kind = classify_renderer_error(&e);
                    last_failover_reason = Some(err_kind.clone());
                    if let Some(k) = trackable {
                        self.breakers.record_result(&host, k, false).await;
                        if k == RendererKind::Lightpanda {
                            let _ = self.preferences.record_failure(&host, &err_kind).await;
                        }
                    }
                    if let Some(g) = probe_guard.take() {
                        g.disarm();
                    }
                    last_error = Some(e);
                    continue;
                }
            }
        }
        // Return the best thin result if we have one, otherwise the last error.
        if let Some(mut result) = thin_result {
            // Stamp routing metadata on the soft-failure result too — callers
            // need to know which chain was attempted for debugging.
            if let Some(last) = chain.last().copied() {
                result.credit_cost = credit_for(last);
                result.render_decision = Some(RenderDecision::Failover {
                    chain: chain.clone(),
                    reason: last_failover_reason
                        .clone()
                        .unwrap_or(FailoverErrorKind::Other),
                });
            }
            // When the user hard-pinned a single renderer and it failed thin,
            // failover never ran — surface an actionable hint so callers (SaaS
            // playground, CLI, MCP) can show a banner instead of silently
            // returning broken markdown with `success: true`.
            if is_user_pinned
                && chain.len() == 1
                && let Some(pinned) = chain.first().copied()
            {
                let reason = last_failover_reason
                    .as_ref()
                    .map(|r| r.as_str())
                    .unwrap_or("unknown");
                let hint = format!(
                    "Pinned renderer '{}' returned a failed render ({}). Content may be unreliable. Retry with renderer=\"chrome\" or omit the renderer field for auto-failover.",
                    pinned.as_str(),
                    reason,
                );
                result.warnings.push(hint);
            }
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

    /// Mock fetcher for unit-testing dispatch logic without real CDP/HTTP.
    struct MockFetcher {
        name: &'static str,
        behavior: MockBehavior,
    }

    #[derive(Clone)]
    enum MockBehavior {
        Ok(String),
        Err(String),
    }

    #[async_trait::async_trait]
    impl PageFetcher for MockFetcher {
        async fn fetch(
            &self,
            url: &str,
            _headers: &HashMap<String, String>,
            _wait_for_ms: Option<u64>,
        ) -> CrwResult<FetchResult> {
            match &self.behavior {
                MockBehavior::Ok(html) => Ok(FetchResult {
                    url: url.to_string(),
                    status_code: 200,
                    html: html.clone(),
                    content_type: Some("text/html".to_string()),
                    raw_bytes: None,
                    rendered_with: Some(self.name.to_string()),
                    elapsed_ms: 0,
                    warning: None,
                    render_decision: None,
                    credit_cost: 0,
                    warnings: Vec::new(),
                }),
                MockBehavior::Err(msg) => Err(CrwError::RendererError(msg.clone())),
            }
        }

        fn name(&self) -> &str {
            self.name
        }
        fn supports_js(&self) -> bool {
            true
        }
        async fn is_available(&self) -> bool {
            true
        }
    }

    fn rich_html(marker: &str) -> String {
        format!(
            "<html><body><article>{}{}</article></body></html>",
            marker,
            "x".repeat(200)
        )
    }

    fn make_renderer_with_mocks(mocks: Vec<Arc<dyn PageFetcher>>) -> FallbackRenderer {
        // Build a real HTTP fetcher (won't be hit when render_js=Some(true)).
        let cfg = base_cfg(RendererMode::None);
        let mut r =
            FallbackRenderer::new(&cfg, "crw-test", None, &StealthConfig::default()).unwrap();
        r.js_renderers = mocks;
        r
    }

    #[tokio::test]
    async fn fetch_with_pinned_renderer_filters_pool() {
        let lp = Arc::new(MockFetcher {
            name: "lightpanda",
            behavior: MockBehavior::Ok(rich_html("LP-")),
        }) as Arc<dyn PageFetcher>;
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Ok(rich_html("CHROME-")),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![lp, chrome]);

        let result = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                Some("chrome"),
            )
            .await
            .unwrap();
        assert!(result.html.contains("CHROME-"), "expected chrome output");
        assert_eq!(result.rendered_with.as_deref(), Some("chrome"));
    }

    #[tokio::test]
    async fn fetch_with_pinned_renderer_unknown_returns_error() {
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Ok(rich_html("CHROME-")),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![chrome]);

        let err = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                Some("lightpanda"),
            )
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("lightpanda") && msg.contains("chrome"),
            "expected error to name pinned + available: {msg}"
        );
    }

    #[tokio::test]
    async fn fetch_with_renderer_auto_uses_full_chain() {
        let lp = Arc::new(MockFetcher {
            name: "lightpanda",
            behavior: MockBehavior::Ok(rich_html("LP-")),
        }) as Arc<dyn PageFetcher>;
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Ok(rich_html("CHROME-")),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![lp, chrome]);

        let result = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                Some("auto"),
            )
            .await
            .unwrap();
        // First renderer in the chain wins when both succeed.
        assert!(result.html.contains("LP-"), "expected lightpanda first");
    }

    #[tokio::test]
    async fn failover_skips_renderer_that_returns_failed_render() {
        // LightPanda returns HTML with a Next.js error boundary marker.
        // The chain must skip it and use Chrome's healthy result.
        let bad_lp_html = format!(
            "<html><body><div id=\"__next-error-0\">{}</div></body></html>",
            "x".repeat(200)
        );
        let lp = Arc::new(MockFetcher {
            name: "lightpanda",
            behavior: MockBehavior::Ok(bad_lp_html),
        }) as Arc<dyn PageFetcher>;
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Ok(rich_html("CHROME-OK")),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![lp, chrome]);

        let result = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                None,
            )
            .await
            .unwrap();
        assert!(result.html.contains("CHROME-OK"));
        assert_eq!(result.rendered_with.as_deref(), Some("chrome"));
    }

    #[tokio::test]
    async fn failover_surfaces_warning_when_only_failed_render_available() {
        // Only LightPanda is configured and it returns a failed render. The
        // call must succeed (best-effort thin_result fallback) but the warning
        // must name the failure so callers can surface it to the user.
        let bad_lp_html = format!(
            "<html><body><div id=\"__next-error-0\">{}</div></body></html>",
            "x".repeat(200)
        );
        let lp = Arc::new(MockFetcher {
            name: "lightpanda",
            behavior: MockBehavior::Ok(bad_lp_html),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![lp]);

        let result = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                None,
            )
            .await
            .unwrap();
        let warning = result.warning.expect("expected warning to be set");
        assert!(
            warning.contains("lightpanda") && warning.contains("nextjs_client_error"),
            "warning should name renderer + reason: {warning}"
        );
    }

    #[tokio::test]
    async fn failover_concats_warnings_across_two_failed_renderers() {
        // Both renderers return failed-render HTML. The fallback `thin_result`
        // should carry warnings from BOTH attempts so debugging captures the
        // full chain, not just the first failure.
        let bad_lp_html = format!(
            "<html><body><div id=\"__next-error-0\">{}</div></body></html>",
            "x".repeat(200)
        );
        let bad_chrome_html = format!(
            "<html><body><div id=\"__next_error__\">{}</div></body></html>",
            "y".repeat(200)
        );
        let lp = Arc::new(MockFetcher {
            name: "lightpanda",
            behavior: MockBehavior::Ok(bad_lp_html),
        }) as Arc<dyn PageFetcher>;
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Ok(bad_chrome_html),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![lp, chrome]);

        let result = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                None,
            )
            .await
            .unwrap();
        let warning = result.warning.expect("expected warning to be set");
        assert!(
            warning.contains("lightpanda") && warning.contains("chrome"),
            "warning should mention both renderers: {warning}"
        );
    }

    #[tokio::test]
    async fn fetch_pinned_renderer_failure_propagates() {
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Err("boom".into()),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![chrome]);

        let err = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                Some("chrome"),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("boom"));
    }

    #[tokio::test]
    async fn auto_promoted_host_tries_chrome_first() {
        // Pre-promote example.com via the preference learner so the loop
        // sorts chrome ahead of lightpanda even though lightpanda was
        // declared first. The first renderer in the executed order wins.
        let lp = Arc::new(MockFetcher {
            name: "lightpanda",
            behavior: MockBehavior::Ok(rich_html("LP-")),
        }) as Arc<dyn PageFetcher>;
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Ok(rich_html("CHROME-")),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![lp, chrome]);

        // Force-promote "example.com" by reaching the failure threshold.
        for _ in 0..3 {
            r.preferences
                .record_failure("example.com", &FailoverErrorKind::NextJsClientError)
                .await;
        }

        let result = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                None,
            )
            .await
            .unwrap();
        assert!(
            result.html.contains("CHROME-"),
            "promoted host should hit chrome first, got: {}",
            &result.html[..80.min(result.html.len())]
        );
        assert_eq!(result.credit_cost, 2, "chrome costs 2 credits");
        assert!(matches!(
            result.render_decision,
            Some(RenderDecision::AutoPromoted {
                chosen: RendererKind::Chrome,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn breaker_skipped_renderer_falls_through_to_next() {
        // Trip the per-host breaker for lightpanda, then verify the loop
        // skips it and uses chrome — without ever calling lightpanda.fetch.
        let lp = Arc::new(MockFetcher {
            name: "lightpanda",
            behavior: MockBehavior::Err("would fire if reached".into()),
        }) as Arc<dyn PageFetcher>;
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Ok(rich_html("CHROME-OK")),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![lp, chrome]);

        // Trip global+host breakers for lightpanda by recording 5 failures
        // at the BreakerRegistry layer (default threshold is 5).
        for _ in 0..5 {
            r.breakers
                .record_result("example.com", RendererKind::Lightpanda, false)
                .await;
        }

        let result = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                None,
            )
            .await
            .unwrap();
        assert!(result.html.contains("CHROME-OK"));
        assert!(matches!(
            result.render_decision,
            Some(RenderDecision::BreakerSkipped {
                skipped: RendererKind::Lightpanda,
                chosen: RendererKind::Chrome
            })
        ));
    }

    #[tokio::test]
    async fn user_pinned_failed_render_emits_warning() {
        // Pin lightpanda. It returns failed-render HTML (Next.js error
        // boundary). Because the user hard-pinned, no failover happens.
        // The thin result must carry an actionable warning so callers can
        // surface it instead of silently returning broken markdown.
        let bad_html = format!(
            "<html><body><div id=\"__next-error-0\">{}</div></body></html>",
            "x".repeat(200)
        );
        let lp = Arc::new(MockFetcher {
            name: "lightpanda",
            behavior: MockBehavior::Ok(bad_html),
        }) as Arc<dyn PageFetcher>;
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Ok(rich_html("CHROME-")),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![lp, chrome]);

        let result = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                Some("lightpanda"),
            )
            .await
            .unwrap();
        let pin_hint = result
            .warnings
            .iter()
            .find(|w| w.starts_with("Pinned renderer 'lightpanda'"));
        assert!(
            pin_hint.is_some(),
            "expected pin-failure hint in warnings, got: {:?}",
            result.warnings
        );
        let hint = pin_hint.unwrap();
        assert!(
            hint.contains("nextJsClientError"),
            "hint should name camelCase reason: {hint}"
        );
        assert!(
            hint.contains("renderer=\"chrome\""),
            "hint should suggest a fix: {hint}"
        );
        // chain stays single-element because user pinned → no chrome attempt
        assert!(matches!(
            result.render_decision,
            Some(RenderDecision::Failover { ref chain, .. }) if chain.len() == 1
        ));
    }

    #[tokio::test]
    async fn user_pinned_decision_records_credit_and_kind() {
        let chrome = Arc::new(MockFetcher {
            name: "chrome",
            behavior: MockBehavior::Ok(rich_html("CHROME-")),
        }) as Arc<dyn PageFetcher>;
        let r = make_renderer_with_mocks(vec![chrome]);
        let result = r
            .fetch(
                "https://example.com",
                &HashMap::new(),
                Some(true),
                None,
                Some("chrome"),
            )
            .await
            .unwrap();
        assert_eq!(result.credit_cost, 2);
        assert!(matches!(
            result.render_decision,
            Some(RenderDecision::UserPinned {
                renderer: RendererKind::Chrome
            })
        ));
    }
}
