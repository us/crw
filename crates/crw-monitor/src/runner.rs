//! Check runner: the self-host control-plane core.
//!
//! Given a monitor + target and the prior snapshots/URL-set, a run:
//! 1. fetches the current page set (scrape per-URL, or crawl-discover),
//! 2. diffs each page against its stored snapshot via the pure [`crw_diff`]
//!    engine → per-page `same`/`changed`,
//! 3. computes **set-level** `new`/`removed` by diffing the current discovered
//!    URL set against the prior set (crawl targets only),
//! 4. applies the **site-down gate** (>80% of known URLs vanished → suppress
//!    mass-removed, mark the check `partial`),
//! 5. optionally runs the LLM judge on changed pages, capped per check.
//!
//! The runner takes a [`PageSource`] so it is testable without a live renderer:
//! the real [`EngineSource`] drives `crw_crawl`, tests supply a fake source.

use crate::config::{MonitorConfig, SITE_DOWN_VANISH_FRACTION};
use crate::types::{
    CheckCounts, CheckResult, CheckStatus, Monitor, MonitorTarget, PageResult, PageStatus,
    TargetKind,
};
use crate::{MonitorError, MonitorResult};
use crw_core::config::LlmConfig;
use crw_core::types::{
    ChangeStatus, ChangeTrackingMode, ChangeTrackingOptions, ChangeTrackingSnapshot,
};
use std::collections::{HashMap, HashSet};

/// One fetched page handed to the diff stage.
#[derive(Debug, Clone)]
pub struct FetchedPage {
    pub url: String,
    pub markdown: String,
    pub json: Option<serde_json::Value>,
    pub content_type: Option<String>,
    /// `Some(msg)` if the fetch failed for this URL.
    pub error: Option<String>,
}

/// A source of current pages for a target. Abstracts the engine so the runner
/// is unit-testable.
#[allow(async_fn_in_trait)]
pub trait PageSource {
    /// Fetch the current pages for a target. For scrape targets this is one
    /// entry per requested URL (errors carried inline); for crawl targets it is
    /// the full discovered set (`CrawlState.data`).
    async fn fetch(&self, target: &MonitorTarget) -> MonitorResult<Vec<FetchedPage>>;
}

/// Inputs the runner needs from the store: the prior snapshot per URL and the
/// prior discovered URL set (for set-level new/removed).
#[derive(Debug, Default, Clone)]
pub struct PriorState {
    /// `url -> previous snapshot`.
    pub snapshots: HashMap<String, ChangeTrackingSnapshot>,
    /// The full set of URLs known at the previous check (crawl targets).
    pub known_urls: HashSet<String>,
}

/// Run one check for a single target.
///
/// `judge_llm` is the operator's LLM config (server `[extraction.llm]` or
/// per-monitor BYOK) used when the monitor has a goal + judge enabled. `None`
/// disables judging regardless of monitor settings.
pub async fn run_check<S: PageSource>(
    monitor: &Monitor,
    target: &MonitorTarget,
    prior: &PriorState,
    source: &S,
    cfg: &MonitorConfig,
    judge_llm: Option<&LlmConfig>,
    now_unix: i64,
) -> MonitorResult<CheckResult> {
    let started_at = now_unix;
    let fetched = source.fetch(target).await?;

    let modes = if monitor.modes.is_empty() {
        vec![ChangeTrackingMode::GitDiff]
    } else {
        monitor.modes.clone()
    };

    // ---- Per-page diff (same/changed/error) ----
    let mut pages: Vec<PageResult> = Vec::with_capacity(fetched.len());
    let mut seen_urls: HashSet<String> = HashSet::new();

    for page in &fetched {
        seen_urls.insert(page.url.clone());

        if let Some(err) = &page.error {
            pages.push(PageResult {
                url: page.url.clone(),
                status: PageStatus::Error,
                content_hash: None,
                change_tracking: None,
                error: Some(err.clone()),
            });
            continue;
        }

        let previous = prior.snapshots.get(&page.url).cloned();
        let opts = ChangeTrackingOptions {
            modes: modes.clone(),
            schema: None,
            prompt: None,
            previous,
            tag: Some(page.url.clone()),
            content_type: page.content_type.clone(),
        };
        let result = crw_diff::compute_change_tracking(
            &opts,
            &page.markdown,
            page.json.as_ref(),
            page.content_type.as_deref(),
        );

        // first_observation (no prior snapshot) maps to set-level `new`;
        // otherwise same/changed straight from opencore's status.
        let status = if result.first_observation {
            PageStatus::New
        } else {
            match result.status {
                ChangeStatus::Same => PageStatus::Same,
                ChangeStatus::Changed => PageStatus::Changed,
            }
        };

        pages.push(PageResult {
            url: page.url.clone(),
            status,
            content_hash: Some(result.content_hash.clone()),
            change_tracking: Some(result),
            error: None,
        });
    }

    // ---- Set-level removed + site-down gate (crawl targets only) ----
    let mut site_down = false;
    if target.kind == TargetKind::Crawl {
        let prior_count = prior.known_urls.len();
        let vanished: Vec<&String> = prior
            .known_urls
            .iter()
            .filter(|u| !seen_urls.contains(*u))
            .collect();

        // Site-down gate: if >80% of previously-known URLs vanished AND we knew
        // a non-trivial set, treat it as a transient site outage — suppress the
        // mass-removed signal and mark the check partial.
        if prior_count > 0 {
            let vanish_fraction = vanished.len() as f64 / prior_count as f64;
            // Only gate when the page set actually shrank toward empty; a site
            // that returned >=1 successful non-error page but lost >80% of URLs
            // is the signal we suppress.
            let successful_now = pages
                .iter()
                .filter(|p| p.status != PageStatus::Error)
                .count();
            if vanish_fraction > SITE_DOWN_VANISH_FRACTION && successful_now < prior_count {
                site_down = true;
            }
        }

        if !site_down {
            for url in vanished {
                pages.push(PageResult {
                    url: url.clone(),
                    status: PageStatus::Removed,
                    content_hash: None,
                    change_tracking: None,
                    error: None,
                });
            }
        }
    }

    // ---- Optional LLM judge on changed pages, capped per check ----
    let judge_on = monitor.judge_enabled && monitor.goal.is_some() && judge_llm.is_some();
    if judge_on {
        let goal = monitor.goal.as_deref().unwrap_or("");
        let llm = judge_llm.unwrap();
        // Indices of changed pages eligible to judge, capped per check.
        let eligible = judge_eligible_indices(&pages, cfg.judge_max_pages_per_check);
        let mut tokens_used: u32 = 0;
        for idx in eligible {
            // Token cap (if configured): stop once exceeded.
            if let Some(max_tokens) = cfg.judge_max_tokens_per_check
                && tokens_used >= max_tokens
            {
                break;
            }
            let url = pages[idx].url.clone();
            let ct = pages[idx].change_tracking.as_mut().unwrap();
            let diff_text = ct.diff.as_ref().and_then(|d| d.text.as_deref());
            let json_diff = ct.diff.as_ref().and_then(|d| d.json.as_ref());
            match crw_extract::judge::judge_change(goal, diff_text, json_diff, llm, None).await {
                Ok(judgment) => {
                    if let Some(usage) = &judgment.llm_usage {
                        tokens_used = tokens_used.saturating_add(usage.total_tokens);
                    }
                    ct.judgment = Some(judgment);
                }
                Err(e) => {
                    tracing::warn!(url = %url, error = %e, "judge failed; storing page unjudged");
                }
            }
        }
    }

    let counts = CheckCounts::tally(&pages);
    let status = if site_down {
        CheckStatus::Partial
    } else {
        CheckStatus::Completed
    };

    Ok(CheckResult {
        id: uuid::Uuid::new_v4().to_string(),
        monitor_id: monitor.id.clone(),
        status,
        started_at,
        completed_at: now_unix,
        site_down,
        pages,
        counts,
    })
}

/// Indices of `Changed` pages eligible to be judged, capped at `max_pages`.
/// Pages beyond the cap are intentionally omitted (stored unjudged).
pub fn judge_eligible_indices(pages: &[PageResult], max_pages: usize) -> Vec<usize> {
    pages
        .iter()
        .enumerate()
        .filter(|(_, p)| p.status == PageStatus::Changed)
        .map(|(i, _)| i)
        .take(max_pages)
        .collect()
}

// ===========================================================================
// Real engine-backed page source
// ===========================================================================

/// A [`PageSource`] backed by the in-process `crw_crawl` primitives.
///
/// Holds an `Arc<FallbackRenderer>` + `AppConfig` (same components the server
/// builds) and drives `scrape_url` per URL for scrape targets, or `run_crawl`
/// for crawl targets, surfacing the full discovered set.
pub struct EngineSource {
    pub config: std::sync::Arc<crw_core::config::AppConfig>,
    pub renderer: std::sync::Arc<crw_renderer::FallbackRenderer>,
    pub unit_deadline_ms: u64,
}

impl EngineSource {
    pub fn new(
        config: std::sync::Arc<crw_core::config::AppConfig>,
        renderer: std::sync::Arc<crw_renderer::FallbackRenderer>,
        cfg: &MonitorConfig,
    ) -> Self {
        Self {
            config,
            renderer,
            unit_deadline_ms: cfg.unit_deadline_ms,
        }
    }

    fn scrape_request(&self, url: &str) -> crw_core::types::ScrapeRequest {
        use crw_core::types::OutputFormat;
        crw_core::types::ScrapeRequest {
            url: url.to_string(),
            formats: vec![OutputFormat::Markdown],
            only_main_content: self.config.extraction.only_main_content,
            render_js: None,
            wait_for: None,
            include_tags: vec![],
            exclude_tags: vec![],
            json_schema: None,
            headers: Default::default(),
            css_selector: None,
            xpath: None,
            chunk_strategy: None,
            query: None,
            filter_mode: None,
            top_k: None,
            proxy: None,
            proxy_list: Vec::new(),
            proxy_rotation: None,
            country: None,
            stealth: None,
            actions: None,
            extract: None,
            llm_api_key: None,
            llm_provider: None,
            llm_model: None,
            base_url: None,
            summary_prompt: None,
            max_content_chars: None,
            renderer: None,
            deadline_ms: Some(self.unit_deadline_ms),
            debug: None,
            change_tracking: None,
            goal: None,
            judge_enabled: None,
            parsers: None,
            screenshot_full_page: false,
        }
    }
}

impl PageSource for EngineSource {
    async fn fetch(&self, target: &MonitorTarget) -> MonitorResult<Vec<FetchedPage>> {
        match target.kind {
            TargetKind::Scrape => {
                let mut out = Vec::with_capacity(target.urls.len());
                let llm = self.config.extraction.llm.as_ref();
                for url in &target.urls {
                    let req = self.scrape_request(url);
                    let deadline = crw_core::Deadline::from_request_ms(self.unit_deadline_ms);
                    match crw_crawl::single::scrape_url(
                        &req,
                        &self.renderer,
                        llm,
                        &self.config.extraction,
                        &self.config.crawler.user_agent,
                        self.config.crawler.stealth.enabled,
                        self.config.renderer.render_js_default,
                        deadline,
                    )
                    .await
                    {
                        Ok(data) => out.push(scrape_to_page(url, data)),
                        Err(e) => out.push(FetchedPage {
                            url: url.clone(),
                            markdown: String::new(),
                            json: None,
                            content_type: None,
                            error: Some(e.to_string()),
                        }),
                    }
                }
                Ok(out)
            }
            TargetKind::Crawl => {
                let crawl_url = target
                    .crawl_url
                    .as_ref()
                    .ok_or_else(|| MonitorError::Invalid("crawl target missing crawlUrl".into()))?;
                let data = self.run_crawl_collect(crawl_url, target.max_pages).await?;
                Ok(data
                    .into_iter()
                    .map(|d| {
                        let url = d.metadata.source_url.clone();
                        scrape_to_page(&url, d)
                    })
                    .collect())
            }
        }
    }
}

impl EngineSource {
    /// Run a crawl to completion and return the discovered `Vec<ScrapeData>`.
    async fn run_crawl_collect(
        &self,
        url: &str,
        max_pages: Option<u32>,
    ) -> MonitorResult<Vec<crw_core::types::ScrapeData>> {
        use crw_core::types::{CrawlRequest, CrawlState, CrawlStatus, OutputFormat};
        use crw_crawl::crawl::{CrawlOptions, run_crawl};

        let req = CrawlRequest {
            url: url.to_string(),
            max_depth: None,
            max_pages,
            formats: vec![OutputFormat::Markdown],
            only_main_content: self.config.extraction.only_main_content,
            json_schema: None,
            render_js: None,
            wait_for: None,
            renderer: None,
            country: None,
            proxy_list: Vec::new(),
            proxy_rotation: None,
        };
        let initial = CrawlState {
            id: uuid::Uuid::new_v4(),
            success: true,
            status: CrawlStatus::InProgress,
            total: 0,
            completed: 0,
            data: vec![],
            error: None,
        };
        let (tx, mut rx) = tokio::sync::watch::channel(initial);

        let renderer = self.renderer.clone();
        let cfg = self.config.clone();
        let user_agent = cfg.crawler.user_agent.clone();
        let llm = cfg.extraction.llm.clone();
        let id = uuid::Uuid::new_v4();
        let handle = tokio::spawn(async move {
            run_crawl(CrawlOptions {
                id,
                req,
                renderer,
                max_concurrency: cfg.crawler.max_concurrency,
                respect_robots: cfg.crawler.respect_robots_txt,
                requests_per_second: cfg.crawler.requests_per_second,
                user_agent: &user_agent,
                state_tx: tx,
                llm_config: llm.as_ref(),
                proxy: cfg.crawler.proxy.clone(),
                jitter_factor: cfg.crawler.stealth.jitter_factor,
                deadline_ms_per_page: cfg.effective_deadline_ms(None, None),
                per_host_max_concurrent: cfg.crawler.per_host_max_concurrent,
            })
            .await;
        });

        // Wait for terminal state.
        loop {
            {
                let state = rx.borrow();
                if matches!(state.status, CrawlStatus::Completed | CrawlStatus::Failed) {
                    let data = state.data.clone();
                    let failed = state.status == CrawlStatus::Failed;
                    let err = state.error.clone();
                    drop(state);
                    handle.abort();
                    if failed && data.is_empty() {
                        return Err(MonitorError::Engine(
                            err.unwrap_or_else(|| "crawl failed".into()),
                        ));
                    }
                    return Ok(data);
                }
            }
            if rx.changed().await.is_err() {
                // Sender dropped without a terminal state: collect what we have.
                let data = rx.borrow().data.clone();
                return Ok(data);
            }
        }
    }
}

fn scrape_to_page(url: &str, data: crw_core::types::ScrapeData) -> FetchedPage {
    FetchedPage {
        url: url.to_string(),
        markdown: data.markdown.unwrap_or_default(),
        json: data.json,
        content_type: data.content_type,
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crw_core::types::ChangeTrackingMode;

    struct FakeSource {
        pages: Vec<FetchedPage>,
    }
    impl PageSource for FakeSource {
        async fn fetch(&self, _t: &MonitorTarget) -> MonitorResult<Vec<FetchedPage>> {
            Ok(self.pages.clone())
        }
    }

    fn monitor(kind_modes: Vec<ChangeTrackingMode>) -> Monitor {
        Monitor {
            id: "m1".into(),
            name: "test".into(),
            status: crate::types::MonitorStatus::Active,
            schedule: "60s".into(),
            modes: kind_modes,
            goal: None,
            judge_enabled: false,
            llm_provider: None,
            llm_api_key: None,
            llm_model: None,
            webhook: None,
            next_run_at: None,
            last_run_at: None,
            created_at: 0,
        }
    }

    fn scrape_target(urls: &[&str]) -> MonitorTarget {
        MonitorTarget {
            id: "t1".into(),
            monitor_id: "m1".into(),
            kind: TargetKind::Scrape,
            urls: urls.iter().map(|s| s.to_string()).collect(),
            crawl_url: None,
            max_pages: None,
        }
    }

    fn crawl_target() -> MonitorTarget {
        MonitorTarget {
            id: "t1".into(),
            monitor_id: "m1".into(),
            kind: TargetKind::Crawl,
            urls: vec![],
            crawl_url: Some("https://ex.com".into()),
            max_pages: None,
        }
    }

    fn page(url: &str, md: &str) -> FetchedPage {
        FetchedPage {
            url: url.into(),
            markdown: md.into(),
            json: None,
            content_type: None,
            error: None,
        }
    }

    fn snap(md: &str) -> ChangeTrackingSnapshot {
        ChangeTrackingSnapshot {
            markdown: Some(md.into()),
            json: None,
            content_hash: crw_diff::snapshot::hash_markdown(md),
            captured_at: None,
        }
    }

    #[tokio::test]
    async fn first_observation_is_new() {
        let m = monitor(vec![ChangeTrackingMode::GitDiff]);
        let t = scrape_target(&["https://ex.com/a"]);
        let src = FakeSource {
            pages: vec![page("https://ex.com/a", "# hello")],
        };
        let r = run_check(
            &m,
            &t,
            &PriorState::default(),
            &src,
            &MonitorConfig::default(),
            None,
            100,
        )
        .await
        .unwrap();
        assert_eq!(r.counts.new, 1);
        assert_eq!(r.pages[0].status, PageStatus::New);
    }

    #[tokio::test]
    async fn mutating_page_is_changed() {
        let m = monitor(vec![ChangeTrackingMode::GitDiff]);
        let t = scrape_target(&["https://ex.com/a"]);
        let mut prior = PriorState::default();
        prior
            .snapshots
            .insert("https://ex.com/a".into(), snap("Price: $19"));
        let src = FakeSource {
            pages: vec![page("https://ex.com/a", "Price: $24")],
        };
        let r = run_check(&m, &t, &prior, &src, &MonitorConfig::default(), None, 100)
            .await
            .unwrap();
        assert_eq!(r.counts.changed, 1);
        assert_eq!(r.pages[0].status, PageStatus::Changed);
        assert!(r.pages[0].change_tracking.as_ref().unwrap().diff.is_some());
    }

    #[tokio::test]
    async fn identical_page_is_same() {
        let m = monitor(vec![ChangeTrackingMode::GitDiff]);
        let t = scrape_target(&["https://ex.com/a"]);
        let mut prior = PriorState::default();
        prior
            .snapshots
            .insert("https://ex.com/a".into(), snap("Price: $19"));
        let src = FakeSource {
            pages: vec![page("https://ex.com/a", "Price: $19")],
        };
        let r = run_check(&m, &t, &prior, &src, &MonitorConfig::default(), None, 100)
            .await
            .unwrap();
        assert_eq!(r.counts.same, 1);
    }

    #[tokio::test]
    async fn set_level_new_and_removed_on_crawl() {
        let m = monitor(vec![ChangeTrackingMode::GitDiff]);
        let t = crawl_target();
        // Prior set knew /a and /b; current discovered set is /a (same) and /c (new).
        let mut prior = PriorState::default();
        prior.known_urls.insert("https://ex.com/a".into());
        prior.known_urls.insert("https://ex.com/b".into());
        prior.snapshots.insert("https://ex.com/a".into(), snap("A"));
        prior.snapshots.insert("https://ex.com/b".into(), snap("B"));
        let src = FakeSource {
            pages: vec![page("https://ex.com/a", "A"), page("https://ex.com/c", "C")],
        };
        let r = run_check(&m, &t, &prior, &src, &MonitorConfig::default(), None, 100)
            .await
            .unwrap();
        // /a same, /c new, /b removed.
        assert_eq!(r.counts.same, 1, "a is same");
        assert_eq!(r.counts.new, 1, "c is new");
        assert_eq!(r.counts.removed, 1, "b removed");
        assert_eq!(r.status, CheckStatus::Completed);
        assert!(!r.site_down);
    }

    #[tokio::test]
    async fn site_down_gate_suppresses_mass_removed() {
        let m = monitor(vec![ChangeTrackingMode::GitDiff]);
        let t = crawl_target();
        let mut prior = PriorState::default();
        for i in 0..10 {
            let u = format!("https://ex.com/{i}");
            prior.known_urls.insert(u.clone());
            prior.snapshots.insert(u, snap("x"));
        }
        // Current discovery returns only 1 of 10 → 90% vanished.
        let src = FakeSource {
            pages: vec![page("https://ex.com/0", "x")],
        };
        let r = run_check(&m, &t, &prior, &src, &MonitorConfig::default(), None, 100)
            .await
            .unwrap();
        assert!(r.site_down);
        assert_eq!(r.status, CheckStatus::Partial);
        assert_eq!(r.counts.removed, 0, "mass-removed suppressed");
    }

    #[tokio::test]
    async fn error_page_recorded() {
        let m = monitor(vec![ChangeTrackingMode::GitDiff]);
        let t = scrape_target(&["https://ex.com/a"]);
        let src = FakeSource {
            pages: vec![FetchedPage {
                url: "https://ex.com/a".into(),
                markdown: String::new(),
                json: None,
                content_type: None,
                error: Some("timeout".into()),
            }],
        };
        let r = run_check(
            &m,
            &t,
            &PriorState::default(),
            &src,
            &MonitorConfig::default(),
            None,
            100,
        )
        .await
        .unwrap();
        assert_eq!(r.counts.error, 1);
        assert_eq!(r.pages[0].status, PageStatus::Error);
    }

    #[test]
    fn judge_cap_limits_eligible_pages() {
        let mk = |status: PageStatus| PageResult {
            url: "u".into(),
            status,
            content_hash: None,
            change_tracking: None,
            error: None,
        };
        // 5 changed pages interleaved with same/new; cap = 2 → only first 2
        // changed indices judged.
        let pages = vec![
            mk(PageStatus::Same),
            mk(PageStatus::Changed), // idx 1
            mk(PageStatus::New),
            mk(PageStatus::Changed), // idx 3
            mk(PageStatus::Changed), // idx 5 dropped
            mk(PageStatus::Changed),
        ];
        let eligible = judge_eligible_indices(&pages, 2);
        assert_eq!(eligible, vec![1, 3]);
        // cap 0 → nothing.
        assert!(judge_eligible_indices(&pages, 0).is_empty());
        // large cap → all changed.
        assert_eq!(judge_eligible_indices(&pages, 99).len(), 4);
    }

    // With no LLM config judging is off; assert judging is skipped without a key.
    #[tokio::test]
    async fn judge_skipped_without_llm() {
        let mut m = monitor(vec![ChangeTrackingMode::GitDiff]);
        m.goal = Some("price changes".into());
        m.judge_enabled = true;
        let t = scrape_target(&["https://ex.com/a"]);
        let mut prior = PriorState::default();
        prior
            .snapshots
            .insert("https://ex.com/a".into(), snap("$19"));
        let src = FakeSource {
            pages: vec![page("https://ex.com/a", "$24")],
        };
        // judge_llm = None → judging disabled regardless of monitor settings.
        let r = run_check(&m, &t, &prior, &src, &MonitorConfig::default(), None, 100)
            .await
            .unwrap();
        assert!(
            r.pages[0]
                .change_tracking
                .as_ref()
                .unwrap()
                .judgment
                .is_none()
        );
    }
}
