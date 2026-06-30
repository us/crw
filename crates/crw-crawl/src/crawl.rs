use crw_core::config::LlmConfig;
use crw_core::error::CrwResult;
use crw_core::types::{
    CrawlRequest, CrawlState, CrawlStatus, RequestedRenderer, ScrapeData, resolve_pinned_renderer,
};
use crw_extract::readability::extract_links;
use crw_renderer::FallbackRenderer;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::robots::RobotsTxt;
use crate::single::derive_target_warning;

/// Default URL discovery limit when a caller doesn't specify one.
pub const DEFAULT_MAX_DISCOVERED_URLS: usize = 5000;
/// Hard ceiling on `DiscoverOptions::max_urls` — caps memory even when a caller
/// asks for "everything" (a site like songsterr.com exposes ~4.3M sitemap URLs;
/// holding them all is a few hundred MB, which is the most we allow per call).
pub const MAX_DISCOVERED_URLS_CEILING: usize = 5_000_000;

/// Options for running a BFS crawl job.
pub struct CrawlOptions<'a> {
    pub id: Uuid,
    pub req: CrawlRequest,
    pub renderer: Arc<FallbackRenderer>,
    pub max_concurrency: usize,
    pub respect_robots: bool,
    pub requests_per_second: f64,
    pub user_agent: &'a str,
    pub state_tx: tokio::sync::watch::Sender<CrawlState>,
    pub llm_config: Option<&'a LlmConfig>,
    /// Proxy URL for the crawler's reqwest client (robots.txt fetching).
    /// Supports HTTP, HTTPS, and SOCKS5
    /// (e.g. `http://proxy:8080` or `socks5://user:pass@proxy:1080`).
    pub proxy: Option<String>,
    /// Jitter factor for rate limiting (0.0–1.0). 0.2 = ±20% of sleep duration.
    pub jitter_factor: f64,
    /// Per-page deadline budget in milliseconds. Each URL fetched in the
    /// crawl gets a fresh `Deadline` of this length.
    pub deadline_ms_per_page: u64,
    /// Cap on concurrent in-flight requests per eTLD+1 host. `1` enforces
    /// strict politeness; raise via config when scraping owned infrastructure.
    pub per_host_max_concurrent: u32,
}

/// Validate that a URL is safe to fetch (scheme + host check).
fn is_safe_url(url: &url::Url) -> bool {
    crw_core::url_safety::validate_safe_url(url).is_ok()
}

/// Send a failed crawl state.
fn send_failed(id: Uuid, state_tx: &tokio::sync::watch::Sender<CrawlState>, error: String) {
    let _ = state_tx.send(CrawlState {
        id,
        success: false,
        status: CrawlStatus::Failed,
        total: 0,
        completed: 0,
        data: vec![],
        error: Some(error),
    });
}

/// Extract same-origin links from a page and enqueue new ones for crawling.
fn enqueue_discovered_links(
    html: &str,
    page_url: &str,
    origin: &str,
    max_pages: usize,
    visited: &mut HashSet<String>,
    queue: &mut VecDeque<(String, u32)>,
    depth: u32,
) {
    let page_links = extract_links(html, page_url);
    for link in page_links {
        if let Ok(link_url) = url::Url::parse(&link) {
            if !is_safe_url(&link_url) {
                continue;
            }
            let link_host = link_url.host_str().unwrap_or("");
            let link_origin = format!("{}://{}", link_url.scheme(), link_host);
            if link_origin != origin {
                continue;
            }
            let normalized = normalize_url(&link);
            if !visited.contains(&normalized) && visited.len() < max_pages {
                visited.insert(normalized.clone());
                queue.push_back((normalized, depth + 1));
            }
        }
    }
}

/// Run a BFS crawl starting from a URL.
pub async fn run_crawl(opts: CrawlOptions<'_>) {
    // Propagate crawl-level country to every page-fetch through the renderer
    // stack via the same task-local used by `single::scrape_url`.
    let country = opts.req.country.clone();
    crw_renderer::REQUEST_COUNTRY
        .scope(country, run_crawl_inner(opts))
        .await
}

async fn run_crawl_inner(opts: CrawlOptions<'_>) {
    let CrawlOptions {
        id,
        req,
        renderer,
        max_concurrency,
        respect_robots,
        requests_per_second,
        user_agent,
        state_tx,
        llm_config,
        proxy,
        jitter_factor: _,
        deadline_ms_per_page,
        per_host_max_concurrent,
    } = opts;

    let max_depth = req.max_depth.unwrap_or(2).min(10);
    let max_pages = req.max_pages.unwrap_or(100).min(1000) as usize;

    // Per-crawl BYOP proxy pool (req.proxy_list, req.proxy_rotation). Takes
    // precedence over the server-config rotator. A malformed entry fails the
    // whole job — never a silent direct connection (real-IP leak).
    let byop_rotator = match crw_core::ProxyRotator::build(
        &req.proxy_list,
        None,
        req.proxy_rotation.unwrap_or_default(),
    ) {
        Ok(r) => r.map(std::sync::Arc::new),
        Err(e) => {
            send_failed(id, &state_tx, format!("invalid proxy_list: {e}"));
            return;
        }
    };

    // Apply "pinned implies JS" once per crawl, mirroring single.rs.
    let pinned_renderer = resolve_pinned_renderer(req.renderer);
    let effective_render_js = if req.renderer.is_some()
        && req.renderer != Some(RequestedRenderer::Auto)
        && req.render_js.is_none()
    {
        Some(true)
    } else {
        req.render_js
    };

    let base_url = match url::Url::parse(&req.url) {
        Ok(u) => {
            if let Err(e) = crw_core::url_safety::validate_safe_url_resolved(&u).await {
                send_failed(id, &state_tx, e);
                return;
            }
            u
        }
        Err(e) => {
            send_failed(id, &state_tx, format!("Invalid URL: {e}"));
            return;
        }
    };

    let origin = match base_url.host_str() {
        Some(host) => format!("{}://{}", base_url.scheme(), host),
        None => {
            send_failed(id, &state_tx, "URL has no host".into());
            return;
        }
    };

    // Robots/sitemap egress must match the page egress: prefer the per-crawl
    // BYOP pool, then the config rotator, then the legacy single `proxy`. Never
    // a silent direct connection when a proxy is configured (real-IP leak).
    let robots_proxy: Option<String> = match &byop_rotator {
        Some(b) => Some(b.pick(base_url.host_str()).raw().to_string()),
        None => renderer
            .pick_proxy_for_url(&origin)
            .map(|e| e.raw().to_string())
            .or_else(|| proxy.clone()),
    }
    // An empty/whitespace single `proxy` means "no proxy configured" (e.g. a
    // present-but-empty CRW_CRAWLER__PROXY or a CLI `--proxy ""`, which bypasses
    // config normalization). Treat it as a direct connection rather than handing
    // "" to reqwest::Proxy::all, which rejects it with "builder error"
    // (issue #154). A genuinely malformed non-empty value still fails closed below.
    .filter(|p| !p.trim().is_empty());
    let mut client_builder = reqwest::Client::builder()
        .user_agent(user_agent)
        .redirect(crw_core::url_safety::safe_redirect_policy());
    if let Some(ref proxy_url) = robots_proxy {
        match reqwest::Proxy::all(proxy_url) {
            Ok(p) => client_builder = client_builder.proxy(p),
            Err(e) => {
                send_failed(
                    id,
                    &state_tx,
                    format!("invalid crawl proxy URL '{proxy_url}': {e}"),
                );
                return;
            }
        }
    }
    let client = client_builder
        .build()
        .expect("reqwest client build should not fail");

    let robots = if respect_robots {
        RobotsTxt::fetch(&origin, &client)
            .await
            .unwrap_or_else(|_| RobotsTxt::parse(""))
    } else {
        RobotsTxt::parse("")
    };

    let semaphore = Arc::new(Semaphore::new(max_concurrency));
    // Key the rate limiter by eTLD+1 so subdomains under the same registered
    // domain (e.g. news.example.com + blog.example.com) share a single
    // limiter rather than each getting their own — otherwise we'd hammer the
    // origin's actual infrastructure at N×rps.
    // Per-host rate limit and concurrency cap are owned by FallbackRenderer
    // via crw_renderer::host_limiter; no need to construct them here.
    let _ = (requests_per_second, per_host_max_concurrent);
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, u32)> = VecDeque::new();
    let mut results: Vec<ScrapeData> = Vec::new();

    queue.push_back((req.url.clone(), 0));
    visited.insert(normalize_url(&req.url));

    while let Some((url, depth)) = queue.pop_front() {
        if results.len() >= max_pages {
            break;
        }

        if let Ok(parsed) = url::Url::parse(&url)
            && !robots.is_allowed(parsed.path())
        {
            tracing::debug!(url, "Blocked by robots.txt");
            continue;
        }

        let _permit = match semaphore.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => {
                tracing::error!("Semaphore closed unexpectedly");
                break;
            }
        };
        // Per-host rate limit + concurrency cap is now owned by
        // FallbackRenderer (see crw_renderer::host_limiter). Acquiring here
        // would double-acquire the same global semaphore and deadlock with
        // the renderer's acquire when per_host_max_concurrent = 1.

        let page_deadline = crw_core::Deadline::from_request_ms(deadline_ms_per_page);
        // Per-page proxy selection (sticky-per-host by default) so each crawled
        // host egresses through its assigned proxy across both the HTTP and
        // JS/CDP paths. BYOP pool (if supplied) takes precedence over config.
        // Scoped per page since hosts vary across the crawl.
        let resolved_proxy = match &byop_rotator {
            Some(b) => {
                let host = url::Url::parse(&url)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_string));
                Some(std::sync::Arc::new(b.pick(host.as_deref()).clone()))
            }
            None => renderer.pick_proxy_for_url(&url),
        };
        let empty_headers: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let fetch_fut = renderer.fetch(
            &url,
            &empty_headers,
            effective_render_js,
            req.wait_for,
            pinned_renderer,
            page_deadline,
        );
        let mut fetch_result = match crw_renderer::REQUEST_PROXY
            .scope(resolved_proxy, fetch_fut)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(url, error = %e, "Crawl: failed to fetch page");
                continue;
            }
        };

        // Extract links for further crawling.
        if depth < max_depth {
            enqueue_discovered_links(
                &fetch_result.html,
                &url,
                &origin,
                max_pages,
                &mut visited,
                &mut queue,
                depth,
            );
        }

        let warning = derive_target_warning(&fetch_result);
        // PDF branch: convert document bytes to markdown instead of running the
        // HTML pipeline (crawl auto-parses PDFs — there is no per-page parsers
        // override on CrawlRequest). Mirrors `single.rs::scrape_url_inner`.
        let mut data = if fetch_result.content_type.as_deref() == Some("application/pdf")
            && fetch_result.raw_bytes.is_some()
        {
            let bytes = fetch_result.raw_bytes.take().unwrap();
            let pdf_req = crw_core::types::ScrapeRequest {
                formats: req.formats.clone(),
                ..Default::default()
            };
            let source = crate::pdf::PdfSource {
                source_url: fetch_result.url.clone(),
                status_code: fetch_result.status_code,
                elapsed_ms: fetch_result.elapsed_ms,
                source_filename: None,
            };
            match crate::pdf::convert_pdf_bytes(bytes, &pdf_req, source).await {
                Ok(data) => data,
                Err(err) => {
                    tracing::warn!(url, error = %err, "Crawl: PDF conversion failed");
                    continue;
                }
            }
        } else {
            // Off-reactor, parallelism-bounded extraction (see
            // `crate::extract_pool`). The owned input lets the CPU-bound
            // `extract()` run on the blocking pool without starving the crawl's
            // async fan-out.
            match crate::extract_pool::extract_offloaded(crw_extract::OwnedExtractInput {
                raw_html: fetch_result.html.clone(),
                source_url: fetch_result.url.clone(),
                status_code: fetch_result.status_code,
                rendered_with: fetch_result.rendered_with.clone(),
                elapsed_ms: fetch_result.elapsed_ms,
                render_decision: fetch_result.render_decision.clone(),
                credit_cost: fetch_result.credit_cost,
                warnings: fetch_result.warnings.clone(),
                formats: req.formats.clone(),
                only_main_content: req.only_main_content,
                include_tags: Vec::new(),
                exclude_tags: Vec::new(),
                css_selector: None,
                xpath: None,
                chunk_strategy: None,
                query: None,
                filter_mode: None,
                top_k: None,
                domain_selectors: None,
                captured_responses: fetch_result.captured_responses.clone(),
                debug: false,
                debug_sink: None,
            })
            .await
            {
                Ok(data) => data,
                Err(err) => {
                    tracing::warn!(url, error = %err, "Crawl: extraction failed");
                    continue;
                }
            }
        };
        data.warning = warning;
        // Surface content type on each discovered page so the SaaS monitor
        // reconciler can hash binary/non-text pages instead of diffing them.
        // The actual change-tracking diff for crawl pages runs SaaS-side via
        // POST /v1/change-tracking/diff, not inline here.
        data.content_type = fetch_result.content_type.clone();

        if let (Some(schema), Some(llm)) = (&req.json_schema, llm_config)
            && let Some(md) = &data.markdown
        {
            match crw_extract::structured::extract_structured_with_usage(md, schema, llm, None)
                .await
            {
                Ok(result) => {
                    data.json = Some(result.value);
                    if data.llm_usage.is_none() {
                        data.llm_usage = result.usage;
                    }
                }
                Err(e) => {
                    tracing::warn!(url = url.as_str(), "Crawl LLM extraction failed: {e}")
                }
            }
        }

        results.push(data);

        // Send progress with empty data to avoid O(N²) cloning.
        // Full data is sent only in the final Completed state.
        let _ = state_tx.send(CrawlState {
            id,
            success: true,
            status: CrawlStatus::InProgress,
            total: visited.len() as u32,
            completed: results.len() as u32,
            data: vec![],
            error: None,
        });
    }

    let _ = state_tx.send(CrawlState {
        id,
        success: true,
        status: CrawlStatus::Completed,
        total: visited.len() as u32,
        completed: results.len() as u32,
        data: results,
        error: None,
    });
}

/// Options for URL discovery (map endpoint).
pub struct DiscoverOptions<'a> {
    pub base_url: &'a str,
    pub max_depth: u32,
    pub use_sitemap: bool,
    /// Run a short-budget BFS crawl after the sitemap phase to fill gaps.
    /// When false, returns sitemap-only results.
    pub crawl_fallback: bool,
    pub renderer: &'a Arc<FallbackRenderer>,
    pub max_concurrency: usize,
    pub requests_per_second: f64,
    pub user_agent: &'a str,
    /// Proxy URL for the discovery client.
    /// Supports HTTP, HTTPS, and SOCKS5
    /// (e.g. `http://proxy:8080` or `socks5://user:pass@proxy:1080`).
    pub proxy: Option<String>,
    /// Per-page deadline budget in milliseconds.
    pub deadline_ms_per_page: u64,
    /// Per-eTLD+1 concurrency cap. See `CrawlOptions::per_host_max_concurrent`.
    pub per_host_max_concurrent: u32,
    /// Optional /map URL filter (Tier A drop + Tier B strip). When `None`,
    /// behaviour is the legacy `normalize_url` pass-through.
    pub url_filter: Option<Arc<crate::url_filter::UrlFilterCfg>>,
    /// Max URLs to discover. Clamped to `[1, MAX_DISCOVERED_URLS_CEILING]`.
    /// Use `DEFAULT_MAX_DISCOVERED_URLS` for the historical behaviour.
    pub max_urls: usize,
}

/// Result of [`discover_urls`]. URLs in `urls` have already passed the
/// /map filter; `dropped_action_count` and `stripped_tracking_count` are
/// the per-request stats the API response surfaces back to callers.
#[derive(Debug, Clone, Default)]
pub struct DiscoverResult {
    pub urls: Vec<String>,
    pub dropped_action_count: usize,
    pub stripped_tracking_count: usize,
}

/// If the sitemap phase yields at least this many URLs and `crawl_fallback`
/// is true, the BFS phase still runs but with a much smaller time budget
/// (we have plenty already; spending the full timeout on slow HTML fetches
/// would burn time for marginal gain).
const SITEMAP_SUFFICIENT_THRESHOLD: usize = 50;
/// Hard ceiling on the BFS crawl phase when sitemap was sufficient.
const BFS_SHORT_BUDGET_SECS: u64 = 30;
/// Recursion depth for the sitemap tree fetch. The per-call fetch *count* is
/// derived from `max_urls` (`sitemap_max_fetches`) so a large `<sitemapindex>`
/// (e.g. ultimate-guitar.com: 84 children; songsterr.com: 854) isn't truncated.
const SITEMAP_MAX_DEPTH: u32 = 3;
/// Per-render deadline for the sitemap anti-bot escalation arm. A Cloudflare
/// managed challenge needs a few seconds of JS execution to clear.
const SITEMAP_ESCALATE_DEADLINE_MS: u64 = 30_000;
/// Hard cap on renderer escalations per map call. Chrome renders cost ~100× a
/// plain GET, so a fully-gated site (every child sitemap behind Cloudflare)
/// can't fan out unbounded; we recover the first N sections within the timeout.
const SITEMAP_ESCALATE_BUDGET: usize = 64;
/// Wall-clock budget for the sitemap phase when escalation is active. Bounds the
/// total time spent solving challenges so the map returns partial results within
/// the caller's request timeout instead of being killed with nothing.
const SITEMAP_PHASE_BUDGET_SECS: u64 = 75;

/// Discover URLs from a site (map endpoint).
pub async fn discover_urls(opts: DiscoverOptions<'_>) -> CrwResult<DiscoverResult> {
    let DiscoverOptions {
        base_url,
        max_depth,
        use_sitemap,
        crawl_fallback,
        renderer,
        max_concurrency,
        requests_per_second,
        user_agent,
        proxy,
        deadline_ms_per_page,
        per_host_max_concurrent,
        url_filter,
        max_urls,
    } = opts;
    // `0` is the documented "unbounded" sentinel (MCP/Firecrawl) → the ceiling.
    let max_urls = if max_urls == 0 {
        MAX_DISCOVERED_URLS_CEILING
    } else {
        max_urls.clamp(1, MAX_DISCOVERED_URLS_CEILING)
    };
    // Each leaf sitemap holds up to ~50k URLs (spec) but commonly ~5k. To reach
    // a large `max_urls` the tree walk must be allowed to fetch enough children:
    // a 4.3M-URL site (songsterr) needs ~900 leaf fetches. Scale the fetch budget
    // with the requested limit, with a small floor so the default stays cheap.
    let sitemap_max_fetches = (max_urls / 1000).clamp(200, 50_000);
    let mut dropped_action_count: usize = 0;
    let mut stripped_tracking_count: usize = 0;
    // Helper closures around the optional filter — None ⇒ legacy pass-through
    // (`normalize_url`). Both increment the stats counters so the API can
    // surface them in the response.
    let filter_raw = |raw: &str, dropped: &mut usize, stripped: &mut usize| -> Option<String> {
        match url_filter.as_deref() {
            Some(cfg) => match crate::url_filter::filter_and_normalize_raw(raw, cfg) {
                Some(s) => {
                    if raw.contains('?')
                        && let Ok(p) = url::Url::parse(raw)
                        && s != normalize_url(raw)
                        && p.query().is_some()
                    {
                        *stripped += 1;
                    }
                    Some(s)
                }
                None => {
                    *dropped += 1;
                    None
                }
            },
            None => Some(normalize_url(raw)),
        }
    };
    let filter_parsed = |parsed: &url::Url,
                         raw: &str,
                         dropped: &mut usize,
                         stripped: &mut usize|
     -> Option<String> {
        match url_filter.as_deref() {
            Some(cfg) => match crate::url_filter::filter_and_normalize_parsed(parsed, raw, cfg) {
                Some(s) => {
                    if parsed.query().is_some() && s != normalize_url(raw) {
                        *stripped += 1;
                    }
                    Some(s)
                }
                None => {
                    *dropped += 1;
                    None
                }
            },
            None => Some(normalize_url(raw)),
        }
    };
    let parsed = url::Url::parse(base_url)
        .map_err(|e| crw_core::error::CrwError::InvalidRequest(format!("Invalid URL: {e}")))?;

    crw_core::url_safety::validate_safe_url_resolved(&parsed)
        .await
        .map_err(crw_core::error::CrwError::InvalidRequest)?;

    // Use the URL's full origin (scheme + host + explicit port). Dropping the
    // port here would silently break sitemap discovery on any non-default-port
    // host, because `fetch_sitemap_tree` filters seeds against the target
    // origin tuple including port.
    if parsed.host_str().is_none() {
        return Err(crw_core::error::CrwError::InvalidRequest(
            "URL has no host".into(),
        ));
    }
    let origin = parsed.origin().ascii_serialization();

    // robots/sitemap egress must match page egress: prefer the config rotator
    // (proxy_list), then the legacy single `proxy`. Fail closed on a bad proxy —
    // never a silent direct connection (real-IP leak).
    let discover_proxy: Option<String> = renderer
        .pick_proxy_for_url(&origin)
        .map(|e| e.raw().to_string())
        .or_else(|| proxy.clone())
        // Empty/whitespace single `proxy` = no proxy configured; never hand "" to
        // reqwest::Proxy::all (it errors with "builder error" — issue #154). A
        // malformed non-empty value still fails closed below. Covers the CLI
        // `--proxy ""` path, which bypasses config-level normalization.
        .filter(|p| !p.trim().is_empty());
    let mut discover_client_builder = reqwest::Client::builder()
        .user_agent(user_agent)
        .timeout(std::time::Duration::from_secs(15))
        .connect_timeout(std::time::Duration::from_secs(5))
        .redirect(crw_core::url_safety::safe_redirect_policy());
    if let Some(ref proxy_url) = discover_proxy {
        let p = reqwest::Proxy::all(proxy_url).map_err(|e| {
            crw_core::error::CrwError::InvalidRequest(format!(
                "invalid proxy URL '{proxy_url}': {e}"
            ))
        })?;
        discover_client_builder = discover_client_builder.proxy(p);
    }
    let client = discover_client_builder
        .build()
        .map_err(|e| crw_core::error::CrwError::Internal(format!("http client build: {e}")))?;

    let mut all_urls: HashSet<String> = HashSet::new();

    if use_sitemap {
        // robots.txt fetch is wrapped in the discover client's 15s/5s timeouts —
        // it can no longer block the entire map call (the original bug).
        let robots = RobotsTxt::fetch(&origin, &client)
            .await
            .unwrap_or_else(|_| RobotsTxt::parse(""));

        let seeds: Vec<String> = if !robots.sitemaps.is_empty() {
            robots.sitemaps.clone()
        } else {
            // robots.txt declared nothing — try the well-known fallback paths.
            // /sitemap.xml first because it's the standard entry point and most
            // CMSes (incl. WordPress 5.5+) 301/302 from there to their canonical
            // sitemap index. HEAD probe filters obvious 404s before bodies fetch.
            // /sitemap.xml is the spec-required canonical path — always try
            // it via GET regardless of the HEAD probe (some CDNs return
            // 405/403 to HEAD but answer GET fine, and we don't want to lose
            // the canonical sitemap just because another fallback responded
            // 200 to HEAD). The other fallbacks are CMS-specific guesses; the
            // HEAD probe avoids paying a body GET on obvious 404s for those.
            let canonical = format!("{origin}/sitemap.xml");
            let probe_candidates = [
                format!("{origin}/wp-sitemap.xml"),
                format!("{origin}/sitemap_index.xml"),
                format!("{origin}/sitemap-index.xml"),
            ];
            let probes = probe_candidates.iter().map(|u| {
                let client = client.clone();
                let u = u.clone();
                async move { (u.clone(), crate::sitemap::head_probe(&u, &client).await) }
            });
            let probe_results = futures::future::join_all(probes).await;
            let mut seeds: Vec<String> = vec![canonical];
            seeds.extend(
                probe_results
                    .into_iter()
                    .filter_map(|(u, ok)| ok.then_some(u)),
            );
            seeds
        };

        // Use the operator's per-host politeness cap for sitemap fanout, not
        // the global `max_concurrency` — a sitemap-index with many children
        // is still N requests against the SAME host, and respecting the
        // configured per-host limit is what an operator who set it expects.
        // `fetch_sitemap_tree` clamps to a small ceiling internally.
        // Anti-bot escalation arm: a sitemap behind a Cloudflare/JS challenge
        // is re-fetched through the renderer (which executes the challenge and
        // returns the real XML). Only wired when a JS renderer is available —
        // otherwise a re-fetch would just hit the same wall. Egress (proxy) is
        // resolved per-URL, identical to the BFS page-fetch path below.
        let escalate_render = move |u: String| -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Option<String>> + Send>,
        > {
            let renderer = renderer.clone();
            Box::pin(async move {
                let empty: HashMap<String, String> = HashMap::new();
                let deadline = crw_core::Deadline::from_request_ms(SITEMAP_ESCALATE_DEADLINE_MS);
                let resolved_proxy = renderer.pick_proxy_for_url(&u);
                let fut = renderer.fetch(&u, &empty, Some(true), None, None, deadline);
                match crw_renderer::REQUEST_PROXY.scope(resolved_proxy, fut).await {
                    Ok(r) => Some(r.html),
                    Err(e) => {
                        tracing::debug!("sitemap escalation render failed for {u}: {e}");
                        None
                    }
                }
            })
        };
        let escalator = renderer.js_capable().then(|| {
            crate::sitemap::SitemapEscalator::new(&escalate_render, SITEMAP_ESCALATE_BUDGET)
        });
        // Wall-clock budget for the sitemap phase — only meaningful when the
        // escalation arm is live (plain-HTTP sitemaps finish in well under it).
        // Without it a fully-gated multi-child index (e.g. UG's 84 children, each
        // behind its own Cloudflare solve) would grind every child to the outer
        // request timeout and return nothing; with it the walk stops and returns
        // whatever it recovered.
        let sitemap_deadline = escalator.as_ref().map(|_| {
            std::time::Instant::now() + std::time::Duration::from_secs(SITEMAP_PHASE_BUDGET_SECS)
        });

        let sitemap_urls = crate::sitemap::fetch_sitemap_tree(
            seeds,
            &parsed,
            &client,
            SITEMAP_MAX_DEPTH,
            sitemap_max_fetches,
            max_urls,
            per_host_max_concurrent as usize,
            escalator.as_ref(),
            sitemap_deadline,
        )
        .await;
        for u in sitemap_urls {
            if all_urls.len() >= max_urls {
                break;
            }
            if let Some(n) = filter_raw(&u, &mut dropped_action_count, &mut stripped_tracking_count)
            {
                all_urls.insert(n);
            }
        }
    }

    // Sitemap-only mode (or sitemap was empty + crawl_fallback is off).
    if !crawl_fallback {
        all_urls.insert(base_url.to_string());
        let mut urls: Vec<String> = all_urls.into_iter().collect();
        urls.sort();
        return Ok(DiscoverResult {
            urls,
            dropped_action_count,
            stripped_tracking_count,
        });
    }

    // Sitemap was rich enough → run BFS with a tight budget. Otherwise let it
    // run to the per-page deadline and the outer route timeout.
    let bfs_deadline_at = if all_urls.len() >= SITEMAP_SUFFICIENT_THRESHOLD {
        tracing::info!(
            sitemap_urls = all_urls.len(),
            "sitemap sufficient, BFS will run with short budget"
        );
        Some(std::time::Instant::now() + std::time::Duration::from_secs(BFS_SHORT_BUDGET_SECS))
    } else {
        None
    };

    let max_depth = max_depth.min(10);
    let semaphore = Arc::new(Semaphore::new(max_concurrency));
    // Per-host limiter is owned by FallbackRenderer (see run_crawl).
    let _ = (requests_per_second, per_host_max_concurrent);
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, u32)> = VecDeque::new();

    queue.push_back((base_url.to_string(), 0));
    visited.insert(normalize_url(base_url));

    while let Some((url, depth)) = queue.pop_front() {
        if visited.len() > max_urls {
            break;
        }
        if let Some(deadline) = bfs_deadline_at
            && std::time::Instant::now() >= deadline
        {
            tracing::info!("BFS short budget exhausted; returning sitemap+partial results");
            break;
        }

        let _permit = match semaphore.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => break,
        };
        // Per-host limiter handled in FallbackRenderer (see run_crawl note).
        if let Ok(parsed) = url::Url::parse(&url)
            && crw_core::url_safety::validate_safe_url_resolved(&parsed)
                .await
                .is_err()
        {
            continue;
        }

        let discover_deadline = crw_core::Deadline::from_request_ms(deadline_ms_per_page);
        // Honor the config rotator so discovery page fetches egress through the
        // proxy (sticky-per-host), not direct. Resolved once into REQUEST_PROXY.
        let resolved_proxy = renderer.pick_proxy_for_url(&url);
        let empty_headers: HashMap<String, String> = HashMap::new();
        // render_js = None (auto), not Some(false): SPAs (Angular/React/Vue)
        // serve an empty app shell over plain HTTP, so HTTP-only discovery finds
        // zero navigational links and /map returns only the seed URL (issue #166).
        // Auto mode lets the renderer escalate thin/SPA shells to a JS render
        // (it stays HTTP-only for static sites), matching /scrape and /crawl.
        let fetch_fut = renderer.fetch(&url, &empty_headers, None, None, None, discover_deadline);
        let fetch = crw_renderer::REQUEST_PROXY
            .scope(resolved_proxy, fetch_fut)
            .await;

        if let Ok(result) = fetch {
            let links = extract_links(&result.html, &url);
            for link in links {
                if let Ok(link_url) = url::Url::parse(&link) {
                    if !is_safe_url(&link_url) {
                        continue;
                    }
                    // Compare full origin (scheme + host + explicit port) so a
                    // non-default-port target (e.g. https://example.com:8443)
                    // doesn't reject its own same-origin links.
                    if link_url.origin().ascii_serialization() != origin {
                        continue;
                    }
                    let normalized = match filter_parsed(
                        &link_url,
                        &link,
                        &mut dropped_action_count,
                        &mut stripped_tracking_count,
                    ) {
                        Some(n) => n,
                        None => continue,
                    };
                    if !visited.contains(&normalized) {
                        visited.insert(normalized.clone());
                        all_urls.insert(normalized.clone());
                        if depth < max_depth {
                            queue.push_back((normalized, depth + 1));
                        }
                    }
                }
            }
        }
    }

    all_urls.insert(base_url.to_string());
    let mut urls: Vec<String> = all_urls.into_iter().collect();
    urls.sort();
    Ok(DiscoverResult {
        urls,
        dropped_action_count,
        stripped_tracking_count,
    })
}

/// Normalize URL by removing fragment and trailing slash.
fn normalize_url(url: &str) -> String {
    let without_fragment = url.split('#').next().unwrap_or(url);
    without_fragment.trim_end_matches('/').to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_url_strips_fragment() {
        assert_eq!(
            normalize_url("https://example.com/page#section"),
            "https://example.com/page"
        );
    }

    #[test]
    fn normalize_url_strips_trailing_slash() {
        assert_eq!(
            normalize_url("https://example.com/page/"),
            "https://example.com/page"
        );
    }

    #[test]
    fn normalize_url_lowercase() {
        assert_eq!(
            normalize_url("HTTPS://EXAMPLE.COM/Page"),
            "https://example.com/page"
        );
    }

    #[test]
    fn normalize_url_combined() {
        assert_eq!(
            normalize_url("https://Example.Com/Path/#fragment"),
            "https://example.com/path"
        );
    }

    #[test]
    fn normalize_url_no_changes_needed() {
        assert_eq!(
            normalize_url("https://example.com/page"),
            "https://example.com/page"
        );
    }

    #[test]
    fn is_safe_url_http() {
        assert!(is_safe_url(&url::Url::parse("http://example.com").unwrap()));
    }

    #[test]
    fn is_safe_url_https() {
        assert!(is_safe_url(
            &url::Url::parse("https://example.com").unwrap()
        ));
    }

    #[test]
    fn is_safe_url_ftp_blocked() {
        assert!(!is_safe_url(&url::Url::parse("ftp://example.com").unwrap()));
    }

    #[test]
    fn is_safe_url_file_blocked() {
        assert!(!is_safe_url(
            &url::Url::parse("file:///etc/passwd").unwrap()
        ));
    }

    #[test]
    fn is_safe_url_data_blocked() {
        assert!(!is_safe_url(
            &url::Url::parse("data:text/html,<h1>x</h1>").unwrap()
        ));
    }

    #[test]
    fn is_safe_url_localhost_blocked() {
        assert!(!is_safe_url(
            &url::Url::parse("http://localhost:8080").unwrap()
        ));
        assert!(!is_safe_url(&url::Url::parse("http://127.0.0.1").unwrap()));
    }

    #[test]
    fn is_safe_url_private_ip_blocked() {
        assert!(!is_safe_url(&url::Url::parse("http://10.0.0.1").unwrap()));
        assert!(!is_safe_url(
            &url::Url::parse("http://192.168.1.1").unwrap()
        ));
        assert!(!is_safe_url(
            &url::Url::parse("http://169.254.169.254").unwrap()
        ));
    }

    /// Simulate the clamping logic from run_crawl
    fn clamp_depth(max_depth: Option<u32>) -> u32 {
        max_depth.unwrap_or(2).min(10)
    }

    fn clamp_pages(max_pages: Option<u32>) -> usize {
        max_pages.unwrap_or(100).min(1000) as usize
    }

    #[test]
    fn crawl_max_depth_capped_at_10() {
        assert_eq!(clamp_depth(Some(100)), 10);
    }

    #[test]
    fn crawl_max_pages_capped_at_1000() {
        assert_eq!(clamp_pages(Some(5000)), 1000);
    }

    #[test]
    fn crawl_default_depth() {
        assert_eq!(clamp_depth(None), 2);
    }

    #[test]
    fn crawl_default_pages() {
        assert_eq!(clamp_pages(None), 100);
    }
}
