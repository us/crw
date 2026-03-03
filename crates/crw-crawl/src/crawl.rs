use crw_core::config::LlmConfig;
use crw_core::error::CrwResult;
use crw_core::types::{CrawlRequest, CrawlState, CrawlStatus, ScrapeData};
use crw_extract::readability::extract_links;
use crw_renderer::FallbackRenderer;
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::time::Instant;
use uuid::Uuid;

use crate::robots::RobotsTxt;

/// Maximum URL discovery limit to prevent memory exhaustion.
const MAX_DISCOVERED_URLS: usize = 5000;

/// Simple rate limiter: ensures minimum interval between requests.
struct RateLimiter {
    min_interval: Duration,
    last_request: Instant,
}

impl RateLimiter {
    fn new(requests_per_second: f64) -> Self {
        if requests_per_second < 0.0 {
            tracing::warn!(
                requests_per_second,
                "Negative requests_per_second value, treating as unlimited"
            );
        }
        let min_interval = if requests_per_second > 0.0 {
            Duration::from_secs_f64(1.0 / requests_per_second)
        } else {
            Duration::ZERO
        };
        Self {
            min_interval,
            last_request: Instant::now() - min_interval,
        }
    }

    async fn wait(&mut self) {
        let elapsed = self.last_request.elapsed();
        if elapsed < self.min_interval {
            tokio::time::sleep(self.min_interval - elapsed).await;
        }
        self.last_request = Instant::now();
    }
}

/// Validate that a URL is safe to fetch (scheme + host check).
fn is_safe_url(url: &url::Url) -> bool {
    crw_core::url_safety::validate_safe_url(url).is_ok()
}

/// Run a BFS crawl starting from a URL.
#[allow(clippy::too_many_arguments)]
pub async fn run_crawl(
    id: Uuid,
    req: CrawlRequest,
    renderer: Arc<FallbackRenderer>,
    max_concurrency: usize,
    respect_robots: bool,
    requests_per_second: f64,
    user_agent: &str,
    state_tx: tokio::sync::watch::Sender<CrawlState>,
    llm_config: Option<&LlmConfig>,
) {
    let max_depth = req.max_depth.unwrap_or(2).min(10);
    let max_pages = req.max_pages.unwrap_or(100).min(1000) as usize;

    let base_url = match url::Url::parse(&req.url) {
        Ok(u) if is_safe_url(&u) => u,
        Ok(_) => {
            let _ = state_tx.send(CrawlState {
                id,
                status: CrawlStatus::Failed,
                total: 0,
                completed: 0,
                data: vec![],
                error: Some("Only http/https URLs are allowed".into()),
            });
            return;
        }
        Err(e) => {
            let _ = state_tx.send(CrawlState {
                id,
                status: CrawlStatus::Failed,
                total: 0,
                completed: 0,
                data: vec![],
                error: Some(format!("Invalid URL: {e}")),
            });
            return;
        }
    };

    let origin = match base_url.host_str() {
        Some(host) => format!("{}://{}", base_url.scheme(), host),
        None => {
            let _ = state_tx.send(CrawlState {
                id,
                status: CrawlStatus::Failed,
                total: 0,
                completed: 0,
                data: vec![],
                error: Some("URL has no host".into()),
            });
            return;
        }
    };

    let client = reqwest::Client::builder()
        .user_agent(user_agent)
        .build()
        .unwrap_or_default();

    let robots = if respect_robots {
        RobotsTxt::fetch(&origin, &client)
            .await
            .unwrap_or_else(|_| RobotsTxt::parse(""))
    } else {
        RobotsTxt::parse("")
    };

    let semaphore = Arc::new(Semaphore::new(max_concurrency));
    let mut rate_limiter = RateLimiter::new(requests_per_second);
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
        rate_limiter.wait().await;

        let fetch_result = match renderer.fetch(&url, &Default::default(), None, None).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(url, error = %e, "Crawl: failed to fetch page");
                continue;
            }
        };

        // Extract links for further crawling.
        if depth < max_depth {
            let page_links = extract_links(&fetch_result.html, &url);
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
                        visited.insert(normalized);
                        queue.push_back((link, depth + 1));
                    }
                }
            }
        }

        let mut data = crw_extract::extract(
            &fetch_result.html,
            &fetch_result.url,
            fetch_result.status_code,
            fetch_result.rendered_with,
            fetch_result.elapsed_ms,
            &req.formats,
            req.only_main_content,
            &[],
            &[],
        );

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
            status: CrawlStatus::InProgress,
            total: visited.len() as u32,
            completed: results.len() as u32,
            data: vec![],
            error: None,
        });
    }

    let _ = state_tx.send(CrawlState {
        id,
        status: CrawlStatus::Completed,
        total: visited.len() as u32,
        completed: results.len() as u32,
        data: results,
        error: None,
    });
}

/// Discover URLs from a site (map endpoint).
pub async fn discover_urls(
    base_url: &str,
    max_depth: u32,
    use_sitemap: bool,
    renderer: &Arc<FallbackRenderer>,
    max_concurrency: usize,
    requests_per_second: f64,
    user_agent: &str,
) -> CrwResult<Vec<String>> {
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

    let client = reqwest::Client::builder()
        .user_agent(user_agent)
        .build()
        .unwrap_or_default();

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
                    all_urls.insert(u);
                }
            }
        }
    }

    let max_depth = max_depth.min(10);
    let semaphore = Arc::new(Semaphore::new(max_concurrency));
    let mut rate_limiter = RateLimiter::new(requests_per_second);
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
        rate_limiter.wait().await;

        let fetch = renderer
            .fetch(&url, &Default::default(), Some(false), None)
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
                        visited.insert(normalized);
                        all_urls.insert(link.clone());
                        if depth < max_depth {
                            queue.push_back((link, depth + 1));
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
        assert!(is_safe_url(&url::Url::parse("https://example.com").unwrap()));
    }

    #[test]
    fn is_safe_url_ftp_blocked() {
        assert!(!is_safe_url(&url::Url::parse("ftp://example.com").unwrap()));
    }

    #[test]
    fn is_safe_url_file_blocked() {
        assert!(!is_safe_url(&url::Url::parse("file:///etc/passwd").unwrap()));
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
        assert!(!is_safe_url(
            &url::Url::parse("http://127.0.0.1").unwrap()
        ));
    }

    #[test]
    fn is_safe_url_private_ip_blocked() {
        assert!(!is_safe_url(
            &url::Url::parse("http://10.0.0.1").unwrap()
        ));
        assert!(!is_safe_url(
            &url::Url::parse("http://192.168.1.1").unwrap()
        ));
        assert!(!is_safe_url(
            &url::Url::parse("http://169.254.169.254").unwrap()
        ));
    }

    #[test]
    fn rate_limiter_zero_rps_no_delay() {
        let limiter = RateLimiter::new(0.0);
        assert_eq!(limiter.min_interval, Duration::ZERO);
    }

    #[test]
    fn rate_limiter_negative_rps_no_panic() {
        // Negative RPS should not panic
        let limiter = RateLimiter::new(-1.0);
        assert_eq!(limiter.min_interval, Duration::ZERO);
    }

    #[test]
    fn rate_limiter_normal_rps() {
        let limiter = RateLimiter::new(10.0);
        assert_eq!(limiter.min_interval, Duration::from_millis(100));
    }

    #[tokio::test]
    async fn rate_limiter_first_call_no_wait() {
        let mut limiter = RateLimiter::new(10.0);
        let start = Instant::now();
        limiter.wait().await;
        let elapsed = start.elapsed();
        // First call should return almost immediately (< 10ms)
        assert!(
            elapsed.as_millis() < 10,
            "First call should not wait, took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn rate_limiter_enforces_interval() {
        let mut limiter = RateLimiter::new(10.0); // 100ms interval
        // First call — no wait
        limiter.wait().await;
        let start = Instant::now();
        // Second call — should wait ~100ms
        limiter.wait().await;
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() >= 80,
            "Second call should wait ~100ms, took {elapsed:?}"
        );
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
