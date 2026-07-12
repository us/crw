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

/// RAII guard that inc/decrements the `crw_batch_pipelines_inflight` gauge for
/// the lifetime of one in-flight batch URL-pipeline.
struct InflightGuard;
impl InflightGuard {
    fn new() -> Self {
        crw_core::metrics::metrics().batch_pipelines_inflight.inc();
        InflightGuard
    }
}
impl Drop for InflightGuard {
    fn drop(&mut self) {
        crw_core::metrics::metrics().batch_pipelines_inflight.dec();
    }
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

/// One URL's extraction outcome. Powers the native `/v1/extract` per-URL array
/// contract (`results:[{url,status,data,error,llmUsage}]`), which sidesteps the
/// FC-legacy last-write-wins merge. `llm_usage` lets the SaaS settle real cost.
///
/// The `basis*` / `llm_input_hash` fields carry per-field evidence and are
/// populated only when the request set `basis: true`. They stay per-URL on
/// purpose: a citation is only meaningful next to the document it came from, so
/// merging them into the FC-legacy flattened `data` object would destroy the
/// attribution.
#[derive(Debug, Clone)]
pub struct UrlResult {
    pub url: String,
    pub status: ExtractStatus,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
    pub llm_usage: Option<crw_core::types::LlmUsage>,
    pub basis: Option<Vec<crw_core::evidence::Basis>>,
    pub basis_warnings: Vec<crw_core::evidence::BasisWarning>,
    pub llm_input_hash: Option<String>,
}

/// A URL prepared by the handler for the worker, in original request order.
/// `preflight_error: Some(..)` marks a parse/SSRF failure that must surface as a
/// `failed` result without being fetched (native contract: no silent drops).
#[derive(Debug, Clone)]
pub struct PreparedUrl {
    pub url: String,
    pub preflight_error: Option<String>,
}

/// An async extract job record. `data` is the single merged JSON object (the
/// scrape's `json` field unioned across URLs), preserved for the FC-legacy
/// `GET /v2/extract/{id}` `data` shape. `per_url` is the native per-URL array
/// (`GET /v1/extract/{id}`), in original request order.
#[derive(Debug, Clone)]
pub struct ExtractRecord {
    pub status: ExtractStatus,
    pub data: Option<serde_json::Value>,
    pub per_url: Vec<UrlResult>,
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
    /// Process-wide cap on the total in-flight `/v2/batch/scrape` URL-pipelines
    /// across all batch-scrape jobs (aggregate bound so `N jobs × width` can't
    /// explode). Targets batch scrape specifically because that's the only wide
    /// fan-out: crawl is BFS-sequential and `/v2/extract` scrapes one URL at a
    /// time, both already bounded by the `crawl_semaphore` job cap. `None` =
    /// unbounded (config `max_aggregate_batch_pipelines = 0`/absent). Acquired
    /// as the first op in each batch URL future, before fetch.
    pub batch_pipeline_sem: Option<Arc<tokio::sync::Semaphore>>,
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
            config.crawler.per_host_interactive_reserve,
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

        // Install the process-wide reserved-lane limits (extract / PDF / LLM)
        // HERE — inside `AppState::new` — so every entry point that builds an
        // AppState (the `crw-server` binary AND `crw serve` / embedded CLI) gets
        // the configured concurrency + reservations, not just the fallbacks.
        // All three are idempotent (first-call-wins).
        let extract_total = config.extraction.max_concurrent_extracts;
        crw_crawl::extract_pool::configure_extract_limit(
            extract_total,
            crw_core::config::resolve_interactive_reserve(
                config.extraction.reserved_interactive_extracts,
                extract_total,
            ),
        );
        crw_crawl::pdf::configure_limits(&config.document);
        if let Some(llm) = &config.extraction.llm {
            crw_extract::llm_gate::configure_llm_limits(
                llm.max_concurrency,
                crw_core::config::resolve_interactive_reserve(
                    llm.reserved_interactive_llm,
                    llm.max_concurrency,
                ),
            );
        }

        // `0`/absent = unbounded aggregate (no cap); any n>0 bounds total
        // in-flight batch URL-pipelines process-wide.
        let batch_pipeline_sem = match config.crawler.max_aggregate_batch_pipelines {
            0 => None,
            n => Some(Arc::new(tokio::sync::Semaphore::new(n))),
        };

        let state = Self {
            config: Arc::new(config),
            renderer: Arc::new(renderer),
            crawl_jobs: Arc::new(RwLock::new(HashMap::new())),
            extract_jobs: Arc::new(RwLock::new(HashMap::new())),
            crawl_semaphore: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_CRAWLS)),
            batch_pipeline_sem,
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
            // Crawl pages are `Batch` traffic (same reserved-lane treatment as
            // batch scrape). Scoped inside the job's spawned task so the
            // task-local reaches every per-page fetch/extract; a handler-level
            // scope would be lost across this `tokio::spawn`.
            crw_core::REQUEST_CLASS
                .scope(crw_core::ScrapeClass::Batch, async {
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
    pub async fn start_batch_job(
        &self,
        urls: Vec<String>,
        template: ScrapeRequest,
        max_concurrency_override: Option<usize>,
    ) -> Uuid {
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
        let batch_pipeline_sem = self.batch_pipeline_sem.clone();
        let config = self.config.clone();
        // Per-job OUTER pipeline width: the SaaS-injected (plan-scaled)
        // `maxConcurrency`, or `max_concurrency` when absent. BOTH paths are
        // clamped to `[1, max_batch_concurrency]` so a batch job never exceeds
        // the ceiling regardless of source (wire value never trusted).
        let width_ceiling = config.crawler.max_batch_concurrency.max(1);
        let max_concurrency = max_concurrency_override
            .unwrap_or(config.crawler.max_concurrency)
            .clamp(1, width_ceiling);

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

            // Stamp every URL in this job as `Batch` traffic. The scope wraps the
            // whole `for_each_concurrent` stream; that combinator polls its
            // futures cooperatively within THIS task (no `tokio::spawn` per URL),
            // so the task-local propagates to each per-URL `scrape_url` and on to
            // the reserved lanes it reads. Scoped here (inside the job's spawned
            // task), not at the handler, because the task-local would be lost
            // across this job's `tokio::spawn`.
            crw_core::REQUEST_CLASS
                .scope(crw_core::ScrapeClass::Batch, async move {
                    futures::stream::iter(reqs)
                        .for_each_concurrent(max_concurrency, |req| {
                            let renderer = renderer.clone();
                            let config = config.clone();
                            let user_agent = user_agent.clone();
                            let tx = tx.clone();
                            let batch_pipeline_sem = batch_pipeline_sem.clone();
                            async move {
                                // Aggregate cap: acquire a process-wide pipeline
                                // permit BEFORE fetching so `N jobs × width` can't
                                // explode. `None` = unbounded. Held for this URL's
                                // whole lifetime.
                                let _pipeline_permit = match &batch_pipeline_sem {
                                    Some(sem) => sem.acquire().await.ok(),
                                    None => None,
                                };
                                // In-flight batch-pipeline gauge (RAII inc/dec).
                                let _inflight = InflightGuard::new();
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
                                // Mutate the shared status in place — push one document
                                // and bump the counter without cloning the whole
                                // accumulated Vec on every completion (avoids O(n^2)
                                // copying on large batches). A failed scrape still
                                // advances `completed`.
                                tx.send_modify(|st| {
                                    if let Some(d) = scraped {
                                        st.data.push(d);
                                    }
                                    st.completed += 1;
                                    // Only flip to Completed from InProgress — never
                                    // overwrite a terminal Cancelled set by DELETE.
                                    if st.completed >= total && st.status == CrawlStatus::InProgress
                                    {
                                        st.status = CrawlStatus::Completed;
                                    }
                                });
                            }
                        })
                        .await;
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

    /// Start an async extract job. Each entry is scraped with `formats:[json]` +
    /// the shared template; per-URL `json` objects are both (a) merged into one
    /// object for the FC-legacy `data` shape and (b) kept as an ordered per-URL
    /// array for the native `/v1/extract` contract. `entries` is in original
    /// request order and may include preflight-failed URLs (surfaced as `failed`
    /// results without being fetched).
    pub async fn start_extract_job(
        &self,
        entries: Vec<PreparedUrl>,
        template: ScrapeRequest,
    ) -> Uuid {
        let id = Uuid::new_v4();
        {
            let mut jobs = self.extract_jobs.write().await;
            jobs.insert(
                id,
                ExtractRecord {
                    status: ExtractStatus::Processing,
                    data: None,
                    per_url: Vec::new(),
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
            // `/v2/extract` is a multi-URL background job — `Batch` traffic, so its
            // scrapes use the batch lanes and don't consume the interactive reserve.
            // Scoped inside the spawned task (a handler-level scope is lost across
            // `tokio::spawn`).
            crw_core::REQUEST_CLASS
                .scope(crw_core::ScrapeClass::Batch, async move {
                    let user_agent = config.crawler.user_agent.clone();
                    let default_stealth =
                        config.crawler.stealth.enabled && config.crawler.stealth.inject_headers;
                    let render_js_default = config.renderer.render_js_default;
                    let deadline_ms =
                        config.effective_deadline_ms(template.deadline_ms, template.wait_for);

                    let mut merged = serde_json::Map::new();
                    let mut per_url: Vec<UrlResult> = Vec::with_capacity(entries.len());
                    let mut tokens = 0u32;
                    let mut credits = 0u32;
                    let mut last_err: Option<String> = None;
                    let mut any_ok = false;

                    for entry in entries {
                        // Preflight-failed URLs (bad parse / SSRF) surface as
                        // `failed` without a fetch — never silently dropped.
                        if let Some(err) = entry.preflight_error {
                            last_err = Some(err.clone());
                            per_url.push(UrlResult {
                                url: entry.url,
                                status: ExtractStatus::Failed,
                                data: None,
                                error: Some(err),
                                llm_usage: None,
                                basis: None,
                                basis_warnings: Vec::new(),
                                llm_input_hash: None,
                            });
                            continue;
                        }

                        let mut req = template.clone();
                        req.url = entry.url.clone();
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
                                if let Some(serde_json::Value::Object(obj)) = &d.json {
                                    for (k, v) in obj {
                                        merged.insert(k.clone(), v.clone());
                                    }
                                }
                                if let Some(usage) = &d.llm_usage {
                                    tokens += usage.total_tokens;
                                }
                                credits += if d.credit_cost == 0 { 1 } else { d.credit_cost };
                                per_url.push(UrlResult {
                                    url: entry.url,
                                    status: ExtractStatus::Completed,
                                    data: d.json,
                                    error: None,
                                    llm_usage: d.llm_usage,
                                    basis: d.basis,
                                    basis_warnings: d.basis_warnings,
                                    llm_input_hash: d.llm_input_hash,
                                });
                            }
                            Err(e) => {
                                let msg = e.to_string();
                                last_err = Some(msg.clone());
                                per_url.push(UrlResult {
                                    url: entry.url,
                                    status: ExtractStatus::Failed,
                                    data: None,
                                    error: Some(msg),
                                    llm_usage: None,
                                    basis: None,
                                    basis_warnings: Vec::new(),
                                    llm_input_hash: None,
                                });
                            }
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
                        rec.per_url = per_url;
                        rec.tokens_used = tokens;
                        // ponytail: 1-credit floor even on an all-failed job —
                        // preflight-failed URLs add 0 (they `continue` before the
                        // tally). SaaS settles the real cost separately.
                        rec.credits_used = credits.max(1);
                    }
                })
                .await;
        });

        id
    }
}
