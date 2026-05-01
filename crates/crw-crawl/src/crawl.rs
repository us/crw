use crw_core::config::LlmConfig;
use crw_core::error::CrwResult;
use crw_core::types::{
    CrawlRequest, CrawlState, CrawlStatus, RequestedRenderer, ScrapeData, resolve_pinned_renderer,
};
use crw_extract::readability::extract_links;
use crw_renderer::FallbackRenderer;
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::robots::RobotsTxt;
use crate::single::derive_target_warning;

/// Maximum URL discovery limit to prevent memory exhaustion.
const MAX_DISCOVERED_URLS: usize = 5000;

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
        Ok(u) if is_safe_url(&u) => u,
        Ok(_) => {
            send_failed(id, &state_tx, "Only http/https URLs are allowed".into());
            return;
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

    let mut client_builder = reqwest::Client::builder()
        .user_agent(user_agent)
        .redirect(crw_core::url_safety::safe_redirect_policy());
    if let Some(ref proxy_url) = proxy {
        if let Ok(p) = reqwest::Proxy::all(proxy_url) {
            client_builder = client_builder.proxy(p);
        } else {
            tracing::warn!("Invalid crawl proxy URL: {proxy_url}");
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
        let fetch_result = match renderer
            .fetch(
                &url,
                &Default::default(),
                effective_render_js,
                req.wait_for,
                pinned_renderer,
                page_deadline,
            )
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(url, error = %e, "Crawl: failed to fetch page");
                continue;
            }
        };

        // PDF routing: skip HTML extraction and link discovery for PDFs.
        #[cfg(feature = "pdf")]
        if fetch_result.content_type.as_deref() == Some("application/pdf")
            && let Some(bytes) = &fetch_result.raw_bytes
        {
            let mut data = match crw_extract::pdf::extract_pdf(
                bytes,
                &fetch_result.url,
                fetch_result.status_code,
                fetch_result.elapsed_ms,
                &req.formats,
            ) {
                Ok(data) => data,
                Err(err) => {
                    tracing::warn!(url, error = %err, "Crawl: PDF extraction failed");
                    continue;
                }
            };

            if let (Some(schema), Some(llm)) = (&req.json_schema, llm_config)
                && let Some(md) = &data.markdown
            {
                match crw_extract::structured::extract_structured(md, schema, llm).await {
                    Ok(json) => data.json = Some(json),
                    Err(e) => {
                        tracing::warn!(url = url.as_str(), "Crawl PDF LLM extraction failed: {e}")
                    }
                }
            }

            results.push(data);

            let _ = state_tx.send(CrawlState {
                id,
                success: true,
                status: CrawlStatus::InProgress,
                total: visited.len() as u32,
                completed: results.len() as u32,
                data: vec![],
                error: None,
            });
            continue;
        }

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
        let mut data = match crw_extract::extract(crw_extract::ExtractOptions {
            raw_html: &fetch_result.html,
            source_url: &fetch_result.url,
            status_code: fetch_result.status_code,
            rendered_with: fetch_result.rendered_with.clone(),
            elapsed_ms: fetch_result.elapsed_ms,
            render_decision: fetch_result.render_decision.clone(),
            credit_cost: fetch_result.credit_cost,
            warnings: fetch_result.warnings.clone(),
            formats: &req.formats,
            only_main_content: req.only_main_content,
            include_tags: &[],
            exclude_tags: &[],
            css_selector: None,
            xpath: None,
            chunk_strategy: None,
            query: None,
            filter_mode: None,
            top_k: None,
        }) {
            Ok(data) => data,
            Err(err) => {
                tracing::warn!(url, error = %err, "Crawl: extraction failed");
                continue;
            }
        };
        data.warning = warning;

        if let (Some(schema), Some(llm)) = (&req.json_schema, llm_config)
            && let Some(md) = &data.markdown
        {
            match crw_extract::structured::extract_structured(md, schema, llm).await {
                Ok(json) => data.json = Some(json),
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
}

/// Discover URLs from a site (map endpoint).
pub async fn discover_urls(opts: DiscoverOptions<'_>) -> CrwResult<Vec<String>> {
    let DiscoverOptions {
        base_url,
        max_depth,
        use_sitemap,
        renderer,
        max_concurrency,
        requests_per_second,
        user_agent,
        proxy,
        deadline_ms_per_page,
        per_host_max_concurrent,
    } = opts;
    let parsed = url::Url::parse(base_url)
        .map_err(|e| crw_core::error::CrwError::InvalidRequest(format!("Invalid URL: {e}")))?;

    if !is_safe_url(&parsed) {
        return Err(crw_core::error::CrwError::InvalidRequest(
            "Only http/https URLs are allowed".into(),
        ));
    }

    let origin = match parsed.host_str() {
        Some(host) => format!("{}://{}", parsed.scheme(), host),
        None => {
            return Err(crw_core::error::CrwError::InvalidRequest(
                "URL has no host".into(),
            ));
        }
    };

    let mut discover_client_builder = reqwest::Client::builder()
        .user_agent(user_agent)
        .redirect(crw_core::url_safety::safe_redirect_policy());
    if let Some(ref proxy_url) = proxy
        && let Ok(p) = reqwest::Proxy::all(proxy_url)
    {
        discover_client_builder = discover_client_builder.proxy(p);
    }
    let client = discover_client_builder
        .build()
        .expect("reqwest client build should not fail");

    let mut all_urls: HashSet<String> = HashSet::new();

    if use_sitemap {
        let robots = RobotsTxt::fetch(&origin, &client)
            .await
            .unwrap_or_else(|_| RobotsTxt::parse(""));
        let sitemap_urls: Vec<String> = if robots.sitemaps.is_empty() {
            vec![format!("{origin}/sitemap.xml")]
        } else {
            robots.sitemaps.clone()
        };

        for sm_url in sitemap_urls {
            if let Ok(urls) = crate::sitemap::fetch_sitemap(&sm_url, &client).await {
                for u in urls {
                    if all_urls.len() >= MAX_DISCOVERED_URLS {
                        break;
                    }
                    // Validate sitemap URLs to prevent SSRF via crafted sitemaps.
                    if let Ok(parsed) = url::Url::parse(&u)
                        && is_safe_url(&parsed)
                    {
                        all_urls.insert(u);
                    }
                }
            }
        }
    }

    let max_depth = max_depth.min(10);
    let semaphore = Arc::new(Semaphore::new(max_concurrency));
    // Per-host limiter is owned by FallbackRenderer (see run_crawl).
    let _ = (requests_per_second, per_host_max_concurrent);
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, u32)> = VecDeque::new();

    queue.push_back((base_url.to_string(), 0));
    visited.insert(normalize_url(base_url));

    while let Some((url, depth)) = queue.pop_front() {
        if visited.len() > MAX_DISCOVERED_URLS {
            break;
        }

        let _permit = match semaphore.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => break,
        };
        // Per-host limiter handled in FallbackRenderer (see run_crawl note).

        let discover_deadline = crw_core::Deadline::from_request_ms(deadline_ms_per_page);
        let fetch = renderer
            .fetch(
                &url,
                &Default::default(),
                Some(false),
                None,
                None,
                discover_deadline,
            )
            .await;

        if let Ok(result) = fetch {
            let links = extract_links(&result.html, &url);
            for link in links {
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
    Ok(urls)
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
