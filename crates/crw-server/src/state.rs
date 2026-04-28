use crw_core::config::AppConfig;
use crw_core::error::{CrwError, CrwResult};
use crw_core::types::{
    CrawlRequest, CrawlState, CrawlStatus, RequestedRenderer, resolve_pinned_renderer,
    resolve_render_js,
};
use crw_crawl::crawl::{CrawlOptions, run_crawl};
use crw_renderer::FallbackRenderer;
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
    pub created_at: Instant,
    /// Handle to abort the crawl task.
    pub abort_handle: Option<tokio::task::AbortHandle>,
}

/// Maximum number of concurrent crawl jobs.
const MAX_CONCURRENT_CRAWLS: usize = 10;
/// Interval between expired crawl job cleanup runs.
const JOB_CLEANUP_INTERVAL: Duration = Duration::from_secs(60);

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub renderer: Arc<FallbackRenderer>,
    pub crawl_jobs: Arc<RwLock<HashMap<Uuid, CrawlJob>>>,
    pub crawl_semaphore: Arc<tokio::sync::Semaphore>,
}

impl AppState {
    pub fn new(config: AppConfig) -> CrwResult<Self> {
        let proxy = config.crawler.proxy.as_deref();
        let renderer = FallbackRenderer::new(
            &config.renderer,
            &config.crawler.user_agent,
            proxy,
            &config.crawler.stealth,
        )?;

        let state = Self {
            config: Arc::new(config),
            renderer: Arc::new(renderer),
            crawl_jobs: Arc::new(RwLock::new(HashMap::new())),
            crawl_semaphore: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_CRAWLS)),
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
                        CrawlStatus::Completed | CrawlStatus::Failed
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
}
