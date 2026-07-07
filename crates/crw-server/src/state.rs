use crw_core::Deadline;
use crw_core::config::AppConfig;
use crw_core::error::{CrwError, CrwResult};
use crw_core::types::{
    CrawlRequest, CrawlState, CrawlStatus, RequestedRenderer, ScrapeRequest,
    resolve_pinned_renderer, resolve_render_js,
};
use crw_crawl::crawl::{CrawlOptions, run_crawl};
use crw_crawl::single::scrape_url;
use crw_renderer::FallbackRenderer;
use crw_search::SearxngClient;
use futures::stream::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, watch};
use uuid::Uuid;

/// Validate that a request's pinned renderer is available before accepting
/// the job. Returns `InvalidRequest` (→ HTTP 400) when the named renderer is
/// not in the configured pool. Skipped when `renderJs:false` is set, since
/// HTTP-only ignores the pin.
///
/// We surface this explicitly (rather than silently falling back to "auto")
/// so users get clear feedback when they ask for a renderer the operator
/// hasn't configured. Sites that fail under one renderer often need a
/// specific other one — silent fallback would leave callers wondering why
/// "chrome" gave them the same broken result as "auto".
pub(crate) fn validate_renderer_pin(
    renderer: Option<RequestedRenderer>,
    render_js: Option<bool>,
    state: &AppState,
) -> CrwResult<()> {
    let Some(name) = resolve_pinned_renderer(renderer) else {
        return Ok(());
    };

    // Mirror the fetch-path resolution at `crw-crawl/src/single.rs:41-50` so
    // validation is consistent with what the actual request does. "Pinned
    // implies JS" — when a renderer is pinned and the request omits
    // `renderJs`, force the request to JS=true so a `render_js_default=false`
    // server config doesn't silently send the request through HTTP-only.
    let effective_request = if render_js.is_none() {
        Some(true)
    } else {
        render_js
    };
    let effective_render_js =
        resolve_render_js(effective_request, state.config.renderer.render_js_default);

    if effective_render_js == Some(false) {
        return Ok(());
    }

    let available = state.renderer.js_renderer_names();
    if !available.contains(&name) {
        return Err(CrwError::InvalidRequest(format!(
            "renderer '{}' not available; configured renderers: [{}]. \
             Update server config or omit the 'renderer' field.",
            name,
            available.join(", ")
        )));
    }
    Ok(())
}

/// Crawl-specific wrapper around [`validate_renderer_pin`].
pub(crate) fn validate_crawl_renderer(req: &CrawlRequest, state: &AppState) -> CrwResult<()> {
    validate_renderer_pin(req.renderer, req.render_js, state)
}

/// Tracks a crawl job receiver + creation time for TTL cleanup.
pub struct CrawlJob {
    pub rx: watch::Receiver<CrawlState>,
    /// Sender kept alongside the receiver so cancel handlers can flip the
    /// job to a terminal `Cancelled` state after aborting the task.
    pub tx: watch::Sender<CrawlState>,
    pub created_at: Instant,
    /// Handle to abort the crawl task.
    pub abort_handle: Option<tokio::task::AbortHandle>,
}

/// Maximum number of concurrent crawl jobs.
const MAX_CONCURRENT_CRAWLS: usize = 10;
/// Interval between expired crawl job cleanup runs.
const JOB_CLEANUP_INTERVAL: Duration = Duration::from_secs(60);

/// Lifecycle of an async `/v2/extract` job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractStatus {
    Processing,
    Completed,
    Failed,
}

impl ExtractStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ExtractStatus::Processing => "processing",
            ExtractStatus::Completed => "completed",
            ExtractStatus::Failed => "failed",
        }
    }
}

/// A `/v2/extract` job record. `data` is the single merged JSON object (the
/// scrape's `json` field unioned across URLs), matching the live API's
/// `GET /v2/extract/{id}` `data` shape (an object, not an array of documents).
#[derive(Debug, Clone)]
pub struct ExtractRecord {
    pub status: ExtractStatus,
    pub data: Option<serde_json::Value>,
    pub tokens_used: u32,
    pub credits_used: u32,
    pub error: Option<String>,
    pub created_at: Instant,
}

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub renderer: Arc<FallbackRenderer>,
    pub crawl_jobs: Arc<RwLock<HashMap<Uuid, CrawlJob>>>,
    /// `/v2/extract` jobs. Separate from `crawl_jobs` because an extract result
    /// is a single merged JSON object, not a `Vec<ScrapeData>`.
    pub extract_jobs: Arc<RwLock<HashMap<Uuid, ExtractRecord>>>,
    pub crawl_semaphore: Arc<tokio::sync::Semaphore>,
    /// SearXNG client. `None` when `[search].searxng_url` is unset, in which
    /// case `/v1/search` returns a clear `search_disabled` error.
    pub searxng: Option<Arc<SearxngClient>>,
    /// Server-wide default /map URL filter. `None` disables the filter
    /// entirely (legacy behaviour). Per-request overrides may swap or
    /// extend this at handler time.
    pub url_filter: Option<Arc<crw_crawl::url_filter::UrlFilterCfg>>,
}

impl AppState {
    pub fn new(config: AppConfig) -> CrwResult<Self> {
        // Build the proxy rotator from config (list takes precedence over the
        // single `proxy`). When present, it owns ALL proxy routing (HTTP pool +
        // per-request CDP proxyServer), so `new()` gets `proxy = None` and the
        // rotator is attached via `with_proxy_rotator`. An invalid proxy URL is
        // a hard startup error — never a silent direct-connection fallback.
        let proxy_rotator = crw_core::ProxyRotator::build(
            &config.crawler.proxy_list,
            config.crawler.proxy.as_deref(),
            config.crawler.proxy_rotation,
        )
        .map_err(CrwError::ConfigError)?
        .map(Arc::new);
        let renderer = FallbackRenderer::new(
            &config.renderer,
            &config.crawler.user_agent,
            None,
            &config.crawler.stealth,
        )?
        .with_proxy_rotator(proxy_rotator)?
        .with_host_limits(
            config.crawler.requests_per_second,
            config.crawler.per_host_max_concurrent,
        );

        let searxng = if config.search.enabled
            && let Some(url) = config.search.searxng_url.as_ref()
        {
            // Dedicated reqwest client for SearXNG so its connection pool is
            // hot and isolated from the renderer / scrape paths. SearXNG runs
            // on the same docker network in the bundled compose so a 5s
            // connect_timeout is generous.
            let http = reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .build()
                .map_err(|e| {
                    CrwError::Internal(format!("failed to build SearXNG http client: {e}"))
                })?;
            let timeout = Duration::from_millis(config.search.timeout_ms);
            Some(Arc::new(SearxngClient::new(Arc::new(http), url, timeout)))
        } else {
            None
        };

        let url_filter_cfg =
            crw_crawl::url_filter::UrlFilterCfg::from_map_config(&config.map.url_filter);
        // One-shot snapshot of how many rules the filter knows about. Helps
        // operators confirm at boot that the deny-lists actually loaded.
        let m = crw_core::metrics::metrics();
        m.map_filter_rules_loaded
            .with_label_values(&["action"])
            .inc_by(
                (crw_crawl::url_filter_data::DEFAULT_ACTION_PARAMS.len()
                    + url_filter_cfg.action_params.len()) as u64,
            );
        m.map_filter_rules_loaded
            .with_label_values(&["tracking"])
            .inc_by(
                (crw_crawl::url_filter_data::DEFAULT_TRACKING_PARAMS.len()
                    + url_filter_cfg.tracking_params.len()) as u64,
            );
        m.map_filter_rules_loaded
            .with_label_values(&["preserve"])
            .inc_by(
                (crw_crawl::url_filter_data::ALWAYS_PRESERVE.len()
                    + url_filter_cfg.preserve_params.len()) as u64,
            );
        m.map_filter_rules_loaded
            .with_label_values(&["host_override"])
            .inc_by(url_filter_cfg.host_overrides.len() as u64);
        let url_filter = Some(Arc::new(url_filter_cfg));

        let state = Self {
            config: Arc::new(config),
            renderer: Arc::new(renderer),
            crawl_jobs: Arc::new(RwLock::new(HashMap::new())),
            extract_jobs: Arc::new(RwLock::new(HashMap::new())),
            crawl_semaphore: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_CRAWLS)),
            searxng,
            url_filter,
        };

        // Wrap the not-yet-returned state in a block to keep the Ok() shape at the end.
        // Spawn background job cleanup task.
        let cleanup_state = state.clone();
        tokio::spawn(async move {
            let ttl = Duration::from_secs(cleanup_state.config.crawler.job_ttl_secs);
            loop {
                tokio::time::sleep(JOB_CLEANUP_INTERVAL).await;
                let mut jobs = cleanup_state.crawl_jobs.write().await;
                let before = jobs.len();
                jobs.retain(|_id, job| {
                    let is_done = matches!(
                        job.rx.borrow().status,
                        CrawlStatus::Completed | CrawlStatus::Failed | CrawlStatus::Cancelled
                    );
                    // Keep if not done, or if done but within TTL.
                    !is_done || job.created_at.elapsed() < ttl
                });
                let removed = before - jobs.len();
                if removed > 0 {
                    tracing::info!(
                        removed,
                        remaining = jobs.len(),
                        "Cleaned up expired crawl jobs"
                    );
                }
                drop(jobs);

                // Prune finished extract jobs past TTL (keep in-flight ones).
                let mut ejobs = cleanup_state.extract_jobs.write().await;
                ejobs.retain(|_id, rec| {
                    matches!(rec.status, ExtractStatus::Processing)
                        || rec.created_at.elapsed() < ttl
                });
            }
        });

        Ok(state)
    }

    /// Start a new crawl job and return its UUID.
    /// Spawns a background task that acquires the crawl semaphore before running.
    pub async fn start_crawl_job(&self, req: CrawlRequest) -> Uuid {
        let id = Uuid::new_v4();
        let initial = CrawlState {
            id,
            success: true,
            status: CrawlStatus::InProgress,
            total: 0,
            completed: 0,
            data: vec![],
            error: None,
        };

        let (tx, rx) = watch::channel(initial);

        {
            let mut jobs = self.crawl_jobs.write().await;
            jobs.insert(
                id,
                CrawlJob {
                    rx,
                    tx: tx.clone(),
                    created_at: Instant::now(),
                    abort_handle: None,
                },
            );
        }

        let renderer = self.renderer.clone();
        let max_concurrency = self.config.crawler.max_concurrency;
        let respect_robots = self.config.crawler.respect_robots_txt;
        let rps = self.config.crawler.requests_per_second;
        let user_agent = self.config.crawler.user_agent.clone();
        let crawl_semaphore = self.crawl_semaphore.clone();
        let llm_config = self.config.extraction.llm.clone();
        let proxy = self.config.crawler.proxy.clone();
        let jitter_factor = self.config.crawler.stealth.jitter_factor;
        let deadline_ms_per_page = self.config.effective_deadline_ms(None, req.wait_for);
        let per_host_max_concurrent = self.config.crawler.per_host_max_concurrent;

        let handle = tokio::spawn(async move {
            let _permit = match crawl_semaphore.acquire().await {
                Ok(p) => p,
                Err(_) => {
                    let _ = tx.send(CrawlState {
                        id,
                        success: false,
                        status: CrawlStatus::Failed,
                        total: 0,
                        completed: 0,
                        data: vec![],
                        error: Some("Server is overloaded, try again later".into()),
                    });
                    return;
                }
            };
            run_crawl(CrawlOptions {
                id,
                req,
                renderer,
                max_concurrency,
                respect_robots,
                requests_per_second: rps,
                user_agent: &user_agent,
                state_tx: tx,
                llm_config: llm_config.as_ref(),
                proxy,
                jitter_factor,
                deadline_ms_per_page,
                per_host_max_concurrent,
            })
            .await;
        });

        // Store the abort handle so the job can be cancelled via DELETE.
        {
            let mut jobs = self.crawl_jobs.write().await;
            if let Some(job) = jobs.get_mut(&id) {
                job.abort_handle = Some(handle.abort_handle());
            }
        }

        id
    }

    /// Start a `/v2/batch/scrape` job over an explicit URL list and return its
    /// UUID. Reuses the crawl-job machinery (`crawl_jobs` + `CrawlState`) but
    /// scrapes the given URLs directly — no link discovery, no same-origin
    /// filtering, no dedup; input order is recoverable via `metadata.sourceURL`.
    pub async fn start_batch_job(&self, urls: Vec<String>, template: ScrapeRequest) -> Uuid {
        let id = Uuid::new_v4();
        let total = urls.len() as u32;
        let (tx, rx) = watch::channel(CrawlState {
            id,
            success: true,
            status: CrawlStatus::InProgress,
            total,
            completed: 0,
            data: vec![],
            error: None,
        });
        {
            let mut jobs = self.crawl_jobs.write().await;
            jobs.insert(
                id,
                CrawlJob {
                    rx,
                    tx: tx.clone(),
                    created_at: Instant::now(),
                    abort_handle: None,
                },
            );
        }

        let renderer = self.renderer.clone();
        let crawl_semaphore = self.crawl_semaphore.clone();
        let config = self.config.clone();
        let max_concurrency = config.crawler.max_concurrency.max(1);

        let handle = tokio::spawn(async move {
            let _permit = match crawl_semaphore.acquire().await {
                Ok(p) => p,
                Err(_) => {
                    let _ = tx.send(CrawlState {
                        id,
                        success: false,
                        status: CrawlStatus::Failed,
                        total,
                        completed: 0,
                        data: vec![],
                        error: Some("Server is overloaded, try again later".into()),
                    });
                    return;
                }
            };

            if total == 0 {
                let _ = tx.send(CrawlState {
                    id,
                    success: true,
                    status: CrawlStatus::Completed,
                    total: 0,
                    completed: 0,
                    data: vec![],
                    error: None,
                });
                return;
            }

            let user_agent = config.crawler.user_agent.clone();
            let default_stealth =
                config.crawler.stealth.enabled && config.crawler.stealth.inject_headers;
            let render_js_default = config.renderer.render_js_default;
            let deadline_ms = config.effective_deadline_ms(template.deadline_ms, template.wait_for);

            let reqs: Vec<ScrapeRequest> = urls
                .into_iter()
                .map(|u| {
                    let mut r = template.clone();
                    r.url = u;
                    r
                })
                .collect();

            futures::stream::iter(reqs)
                .for_each_concurrent(max_concurrency, |req| {
                    let renderer = renderer.clone();
                    let config = config.clone();
                    let user_agent = user_agent.clone();
                    let tx = tx.clone();
                    async move {
                        let deadline = Deadline::from_request_ms(deadline_ms);
                        let scraped = scrape_url(
                            &req,
                            &renderer,
                            config.extraction.llm.as_ref(),
                            &config.extraction,
                            &user_agent,
                            default_stealth,
                            render_js_default,
                            deadline,
                        )
                        .await
                        .ok();
                        // Mutate the shared status in place — push one document and
                        // bump the counter without cloning the whole accumulated Vec
                        // on every completion (avoids O(n^2) copying on large
                        // batches). A failed scrape still advances `completed`.
                        tx.send_modify(|st| {
                            if let Some(d) = scraped {
                                st.data.push(d);
                            }
                            st.completed += 1;
                            // Only flip to Completed from InProgress — never
                            // overwrite a terminal Cancelled set by DELETE.
                            if st.completed >= total && st.status == CrawlStatus::InProgress {
                                st.status = CrawlStatus::Completed;
                            }
                        });
                    }
                })
                .await;
        });

        {
            let mut jobs = self.crawl_jobs.write().await;
            if let Some(job) = jobs.get_mut(&id) {
                job.abort_handle = Some(handle.abort_handle());
            }
        }

        id
    }

    /// Start a `/v2/extract` job. Scrapes each URL with `formats:[json]` + the
    /// shared schema (already set on `template`) and merges the per-URL `json`
    /// objects into one — matching the live API's single-object `data` shape.
    pub async fn start_extract_job(&self, urls: Vec<String>, template: ScrapeRequest) -> Uuid {
        let id = Uuid::new_v4();
        {
            let mut jobs = self.extract_jobs.write().await;
            jobs.insert(
                id,
                ExtractRecord {
                    status: ExtractStatus::Processing,
                    data: None,
                    tokens_used: 0,
                    credits_used: 0,
                    error: None,
                    created_at: Instant::now(),
                },
            );
        }

        let renderer = self.renderer.clone();
        let config = self.config.clone();
        let extract_jobs = self.extract_jobs.clone();

        tokio::spawn(async move {
            let user_agent = config.crawler.user_agent.clone();
            let default_stealth =
                config.crawler.stealth.enabled && config.crawler.stealth.inject_headers;
            let render_js_default = config.renderer.render_js_default;
            let deadline_ms = config.effective_deadline_ms(template.deadline_ms, template.wait_for);

            let mut merged = serde_json::Map::new();
            let mut tokens = 0u32;
            let mut credits = 0u32;
            let mut last_err: Option<String> = None;
            let mut any_ok = false;

            for u in urls {
                let mut req = template.clone();
                req.url = u;
                let deadline = Deadline::from_request_ms(deadline_ms);
                match scrape_url(
                    &req,
                    &renderer,
                    config.extraction.llm.as_ref(),
                    &config.extraction,
                    &user_agent,
                    default_stealth,
                    render_js_default,
                    deadline,
                )
                .await
                {
                    Ok(d) => {
                        any_ok = true;
                        if let Some(serde_json::Value::Object(obj)) = d.json {
                            for (k, v) in obj {
                                merged.insert(k, v);
                            }
                        }
                        if let Some(usage) = d.llm_usage {
                            tokens += usage.total_tokens;
                        }
                        credits += if d.credit_cost == 0 { 1 } else { d.credit_cost };
                    }
                    Err(e) => last_err = Some(e.to_string()),
                }
            }

            let mut jobs = extract_jobs.write().await;
            if let Some(rec) = jobs.get_mut(&id) {
                if !any_ok && last_err.is_some() {
                    rec.status = ExtractStatus::Failed;
                    rec.error = last_err;
                } else {
                    rec.status = ExtractStatus::Completed;
                    rec.data = Some(serde_json::Value::Object(merged));
                }
                rec.tokens_used = tokens;
                rec.credits_used = credits.max(1);
            }
        });

        id
    }
}
