use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use crw_core::Deadline;
use crw_core::config::LlmConfig;
use crw_core::error::CrwError;
use crw_core::types::{
    ApiResponse, OutputFormat, ScrapeData, ScrapeRequest, SearchData, SearchRequest,
    SearchResponse, SearchResponseData, SearchResult, SearchScrapeOptions,
};
use crw_crawl::single::scrape_url;
use crw_extract::answer;
use crw_extract::summary;
use crw_search::{SearchError, map_to_searxng_params, transform_flat, transform_grouped};
use futures::stream::{self, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::task::JoinSet;

const DEFAULT_ANSWER_TOP_N: u32 = 5;
const MAX_ANSWER_TOP_N: u32 = 10;
const DEFAULT_MAX_CHARS_PER_SOURCE: usize = 8192;

use crate::error::AppError;
use crate::state::AppState;

const MAX_QUERY_CHARS: usize = 2000;

/// `POST /v1/search` — search the web via SearXNG, optionally enriching
/// each `web` result by running it through the scrape pipeline in-process.
///
/// Mirrors the public contract exposed by `crw-saas/src/app/api/v1/search/route.ts`
/// (minus the credit / quota wrapper, which lives in the SaaS layer).
pub async fn search(
    State(state): State<AppState>,
    body: Result<Json<SearchRequest>, JsonRejection>,
) -> Result<Json<SearchResponse>, AppError> {
    let Json(req) = body.map_err(AppError::from)?;
    let resp = search_inner(&state, req).await?;
    Ok(Json(resp))
}

/// Shared search logic used by both the HTTP route and the MCP tool dispatcher.
/// Returns the public `SearchResponse` envelope (with `.warning` populated when
/// scrape enrichment partially fails) or a `CrwError` on hard failure.
pub async fn search_inner(
    state: &AppState,
    req: SearchRequest,
) -> Result<SearchResponse, CrwError> {
    validate_request(&req, state.config.search.max_limit)?;

    let client = state
        .searxng
        .as_ref()
        .ok_or_else(|| {
            CrwError::SearchDisabled(
                "Search is disabled. Set [search].searxng_url in config or define \
                 CRW_SEARCH__SEARXNG_URL to point at a SearXNG instance."
                    .into(),
            )
        })?
        .clone();

    let limit = req
        .limit
        .unwrap_or(state.config.search.default_limit)
        .min(state.config.search.max_limit)
        .max(1);

    let params = map_to_searxng_params(&req, &state.config.search);
    let response = client
        .fetch(&params)
        .await
        .map_err(|e| map_search_error(e, state.config.search.timeout_ms))?;

    let has_sources = req.sources.as_ref().is_some_and(|s| !s.is_empty());
    let mut data = if has_sources {
        let sources = req.sources.clone().unwrap_or_default();
        SearchData::Grouped(transform_grouped(&response, &sources, limit))
    } else {
        SearchData::Flat(transform_flat(&response, limit))
    };

    let mut warning: Option<String> = None;
    let mut warnings: Vec<String> = Vec::new();
    if let Some(opts) = req.scrape_options.as_ref() {
        match enrich_with_scrape(&mut data, opts, state).await {
            Ok(()) => {}
            Err(msg) => {
                tracing::warn!(error = %msg, "scrape enrichment failed");
                warning = Some(msg);
            }
        }
    }

    // BYOK + LLM features. Both `summarize_results` and `answer` need an
    // effective LlmConfig. Build it once from the BYOK request fields,
    // falling back to server config.
    let server_llm = state.config.extraction.llm.clone();
    let byok_llm = build_byok_search_llm_config(&req, server_llm.as_ref());
    let effective_llm = byok_llm.as_ref().or(server_llm.as_ref());

    let wants_summaries = req.summarize_results.unwrap_or(false);
    let wants_answer = req.answer.unwrap_or(false);

    if (wants_summaries || wants_answer) && req.scrape_options.is_none() {
        warnings.push(
            "summarizeResults / answer require scrapeOptions to populate markdown; skipped".into(),
        );
    } else if wants_summaries || wants_answer {
        match effective_llm {
            None => warnings.push(
                "summarizeResults / answer require an LLM config (set [extraction.llm] or \
                 pass llm_api_key)"
                    .into(),
            ),
            Some(llm) => {
                if wants_summaries {
                    let count = attach_result_summaries(
                        &mut data,
                        llm,
                        llm.max_concurrency,
                        req.summary_prompt.as_deref(),
                        req.max_content_chars,
                    )
                    .await;
                    if count.failed > 0 {
                        warnings.push(format!(
                            "{} of {} per-result summaries failed",
                            count.failed,
                            count.failed + count.ok
                        ));
                    }
                }
                if wants_answer {
                    match synthesize_answer(&req, &data, llm).await {
                        Ok((ans, cites, usage, mut ans_warns)) => {
                            warnings.append(&mut ans_warns);
                            let wrapped = SearchResponseData {
                                results: data,
                                answer: Some(ans),
                                citations: cites,
                                llm_usage: usage,
                                warnings,
                            };
                            let mut resp = ApiResponse::ok(wrapped);
                            resp.warning = warning;
                            return Ok(resp);
                        }
                        Err(msg) => {
                            tracing::warn!(error = %msg, "answer synthesis failed");
                            warnings.push(format!("answer synthesis failed: {msg}"));
                        }
                    }
                }
            }
        }
    }

    let wrapped = SearchResponseData {
        results: data,
        answer: None,
        citations: Vec::new(),
        llm_usage: None,
        warnings,
    };
    let mut resp = ApiResponse::ok(wrapped);
    resp.warning = warning;
    Ok(resp)
}

#[derive(Default)]
struct SummaryFanoutCount {
    ok: usize,
    failed: usize,
}

/// Fan-out summary calls across all results that have markdown. Bounded by
/// `max_concurrency`. Pattern mirrors `crates/crw-crawl/src/sitemap.rs`.
async fn attach_result_summaries(
    data: &mut SearchData,
    cfg: &LlmConfig,
    max_concurrency: usize,
    user_prompt: Option<&str>,
    max_content_chars: Option<usize>,
) -> SummaryFanoutCount {
    let targets: &mut Vec<SearchResult> = match data {
        SearchData::Flat(v) => v,
        SearchData::Grouped(g) => match g.web.as_mut() {
            Some(v) if !v.is_empty() => v,
            _ => return SummaryFanoutCount::default(),
        },
    };
    // Capture markdown + index pairs first so we don't hold a borrow of
    // `targets` across the async fan-out.
    let jobs: Vec<(usize, String)> = targets
        .iter()
        .enumerate()
        .filter_map(|(idx, r)| r.markdown.as_ref().map(|md| (idx, md.clone())))
        .collect();
    if jobs.is_empty() {
        return SummaryFanoutCount::default();
    }
    let cfg_owned = cfg.clone();
    let user_prompt_owned: Option<String> = user_prompt.map(str::to_owned);
    let concurrency = max_concurrency.max(1);
    let results: Vec<(usize, Result<String, String>)> = stream::iter(jobs)
        .map(|(idx, md)| {
            let cfg = cfg_owned.clone();
            let user_prompt = user_prompt_owned.clone();
            async move {
                let outcome =
                    summary::summarize(&md, &cfg, user_prompt.as_deref(), max_content_chars)
                        .await
                        .map(|r| r.content)
                        .map_err(|e| e.to_string());
                (idx, outcome)
            }
        })
        .buffer_unordered(concurrency)
        .collect()
        .await;

    let mut count = SummaryFanoutCount::default();
    for (idx, res) in results {
        match res {
            Ok(text) => {
                if let Some(slot) = targets.get_mut(idx) {
                    slot.summary = Some(text);
                    count.ok += 1;
                }
            }
            Err(_) => count.failed += 1,
        }
    }
    count
}

async fn synthesize_answer(
    req: &SearchRequest,
    data: &SearchData,
    cfg: &LlmConfig,
) -> Result<
    (
        String,
        Vec<crw_core::types::Citation>,
        Option<crw_core::types::LlmUsage>,
        Vec<String>,
    ),
    String,
> {
    let top_n = req
        .answer_top_n
        .unwrap_or(DEFAULT_ANSWER_TOP_N)
        .min(MAX_ANSWER_TOP_N) as usize;
    let cap = req
        .max_chars_per_source
        .unwrap_or(DEFAULT_MAX_CHARS_PER_SOURCE)
        .min(answer::MAX_CHARS_PER_SOURCE_CEILING);

    let pool: &Vec<SearchResult> = match data {
        SearchData::Flat(v) => v,
        SearchData::Grouped(g) => match g.web.as_ref() {
            Some(v) => v,
            None => return Err("no web results to synthesize from".into()),
        },
    };
    let sources: Vec<answer::Source> = pool
        .iter()
        .filter_map(|r| {
            r.markdown
                .as_ref()
                .map(|md| (r.url.clone(), r.title.clone(), md.clone()))
        })
        .take(top_n)
        .collect();
    if sources.is_empty() {
        return Err("no results carry markdown to synthesize an answer from".into());
    }
    let result = answer::synthesize(&req.query, &sources, cfg, cap, req.answer_prompt.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    Ok((
        result.content,
        result.citations,
        result.usage,
        result.warnings,
    ))
}

fn build_byok_search_llm_config(
    req: &SearchRequest,
    server_cfg: Option<&LlmConfig>,
) -> Option<LlmConfig> {
    let api_key = req.llm_api_key.as_ref()?.clone();
    let mut cfg = match server_cfg {
        Some(s) => s.clone(),
        None => LlmConfig::default(),
    };
    cfg.api_key = api_key;
    if let Some(p) = &req.llm_provider {
        cfg.provider = p.clone();
    }
    if let Some(m) = &req.llm_model {
        cfg.model = m.clone();
    }
    if let Some(b) = &req.base_url {
        cfg.base_url = Some(b.clone());
    }
    Some(cfg)
}

fn validate_request(req: &SearchRequest, max_limit: u32) -> Result<(), CrwError> {
    let len = req.query.chars().count();
    if len == 0 {
        return Err(CrwError::InvalidRequest("query is required".into()));
    }
    if len > MAX_QUERY_CHARS {
        return Err(CrwError::InvalidRequest(format!(
            "query length {len} exceeds maximum of {MAX_QUERY_CHARS} characters"
        )));
    }
    if let Some(l) = req.limit
        && (l == 0 || l > max_limit)
    {
        return Err(CrwError::InvalidRequest(format!(
            "limit must be between 1 and {max_limit} (got {l})"
        )));
    }
    if let Some(cats) = &req.categories
        && cats.len() > 5
    {
        return Err(CrwError::InvalidRequest(
            "categories accepts at most 5 entries".into(),
        ));
    }
    if let Some(opts) = req.scrape_options.as_ref() {
        // Search enrichment can only carry formats that fit the
        // `SearchResult` shape. `plainText` and `json` (LLM extract) require
        // fields the search-result envelope doesn't expose; rejecting up-front
        // is clearer than silently dropping them post-scrape.
        for f in &opts.formats {
            if matches!(f, OutputFormat::PlainText | OutputFormat::Json) {
                return Err(CrwError::InvalidRequest(format!(
                    "scrapeOptions.formats does not support {f:?} on /v1/search; use \
                     /v1/scrape for plainText/json (extract). Allowed: markdown, html, \
                     rawHtml, links."
                )));
            }
        }
    }
    Ok(())
}

fn map_search_error(err: SearchError, timeout_ms: u64) -> CrwError {
    match err {
        SearchError::Timeout => CrwError::Timeout(timeout_ms),
        SearchError::Upstream { status, body } => CrwError::HttpError(format!(
            "SearXNG returned HTTP {status}: {}",
            body.chars().take(200).collect::<String>()
        )),
        SearchError::InvalidResponse(msg) => {
            CrwError::HttpError(format!("SearXNG returned invalid JSON: {msg}"))
        }
        SearchError::Transport(msg) => CrwError::TargetUnreachable(format!("SearXNG: {msg}")),
    }
}

/// Enrich `web` (or flat) results in-place by calling the scrape pipeline
/// for each result URL. Bounded by `[crawler].max_concurrency`. On per-URL
/// failure the result is left without `markdown`/`html`/etc. fields — the
/// search response still succeeds.
async fn enrich_with_scrape(
    data: &mut SearchData,
    opts: &SearchScrapeOptions,
    state: &AppState,
) -> Result<(), String> {
    let targets: &mut Vec<SearchResult> = match data {
        SearchData::Flat(v) => v,
        SearchData::Grouped(g) => match g.web.as_mut() {
            Some(v) if !v.is_empty() => v,
            _ => return Ok(()), // nothing to enrich
        },
    };
    if targets.is_empty() {
        return Ok(());
    }

    // Validate each URL and remember which slot it came from.
    let mut jobs: Vec<(usize, String)> = Vec::new();
    for (idx, r) in targets.iter().enumerate() {
        let parsed = match url::Url::parse(&r.url) {
            Ok(u) => u,
            Err(_) => continue,
        };
        if crw_core::url_safety::validate_safe_url_resolved(&parsed)
            .await
            .is_err()
        {
            continue;
        }
        jobs.push((idx, r.url.clone()));
    }
    if jobs.is_empty() {
        return Ok(());
    }

    let formats = opts.formats.clone();
    let only_main = opts.only_main_content;
    let semaphore = Arc::new(tokio::sync::Semaphore::new(
        state.config.crawler.max_concurrency.max(1),
    ));
    let mut set: JoinSet<(usize, Result<ScrapeData, String>)> = JoinSet::new();

    for (idx, url) in jobs {
        let formats = formats.clone();
        let renderer = state.renderer.clone();
        let llm_config = state.config.extraction.llm.clone();
        let extraction_cfg = state.config.extraction.clone();
        let user_agent = state.config.crawler.user_agent.clone();
        let default_stealth =
            state.config.crawler.stealth.enabled && state.config.crawler.stealth.inject_headers;
        let render_js_default = state.config.renderer.render_js_default;
        let deadline_ms = state.config.effective_deadline_ms(None, None);
        let permit_src = semaphore.clone();

        set.spawn(async move {
            let _permit = match permit_src.acquire_owned().await {
                Ok(p) => p,
                Err(e) => return (idx, Err(format!("semaphore closed: {e}"))),
            };
            let scrape_req = ScrapeRequest {
                url: url.clone(),
                formats,
                only_main_content: only_main,
                render_js: None,
                wait_for: None,
                include_tags: vec![],
                exclude_tags: vec![],
                json_schema: None,
                headers: HashMap::new(),
                css_selector: None,
                xpath: None,
                chunk_strategy: None,
                query: None,
                filter_mode: None,
                top_k: None,
                proxy: None,
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
                deadline_ms: Some(deadline_ms),
                debug: None,
            };
            let deadline = Deadline::from_request_ms(deadline_ms);
            let result = scrape_url(
                &scrape_req,
                &renderer,
                llm_config.as_ref(),
                &extraction_cfg,
                &user_agent,
                default_stealth,
                render_js_default,
                deadline,
            )
            .await
            .map_err(|e| e.to_string());
            (idx, result)
        });
    }

    while let Some(joined) = set.join_next().await {
        let (idx, result) = match joined {
            Ok(pair) => pair,
            Err(join_err) => {
                tracing::warn!(error = %join_err, "scrape enrichment task panicked");
                continue;
            }
        };
        let Some(slot) = targets.get_mut(idx) else {
            continue;
        };
        match result {
            Ok(scrape) => apply_scrape_to_result(slot, scrape, &opts.formats),
            Err(msg) => {
                tracing::debug!(url = %slot.url, error = %msg, "scrape enrichment skipped");
            }
        }
    }
    Ok(())
}

fn apply_scrape_to_result(slot: &mut SearchResult, data: ScrapeData, formats: &[OutputFormat]) {
    if formats.contains(&OutputFormat::Markdown) {
        slot.markdown = data.markdown;
    }
    if formats.contains(&OutputFormat::Html) {
        slot.html = data.html;
    }
    if formats.contains(&OutputFormat::RawHtml) {
        slot.raw_html = data.raw_html;
    }
    if formats.contains(&OutputFormat::Links) {
        slot.links = data.links;
    }
    slot.metadata = Some(data.metadata);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crw_core::types::SearchSource;

    fn req(q: &str) -> SearchRequest {
        SearchRequest {
            query: q.into(),
            limit: None,
            lang: None,
            tbs: None,
            sources: None,
            categories: None,
            scrape_options: None,
            summarize_results: None,
            answer: None,
            answer_top_n: None,
            max_chars_per_source: None,
            llm_api_key: None,
            llm_provider: None,
            llm_model: None,
            base_url: None,
            summary_prompt: None,
            answer_prompt: None,
            max_content_chars: None,
        }
    }

    #[test]
    fn validate_rejects_empty_query() {
        assert!(matches!(
            validate_request(&req(""), 20),
            Err(CrwError::InvalidRequest(_))
        ));
    }

    #[test]
    fn validate_rejects_oversized_query() {
        let q = "x".repeat(MAX_QUERY_CHARS + 1);
        assert!(matches!(
            validate_request(&req(&q), 20),
            Err(CrwError::InvalidRequest(_))
        ));
    }

    #[test]
    fn validate_rejects_limit_above_max() {
        let mut r = req("rust");
        r.limit = Some(50);
        assert!(matches!(
            validate_request(&r, 20),
            Err(CrwError::InvalidRequest(_))
        ));
    }

    #[test]
    fn validate_rejects_zero_limit() {
        let mut r = req("rust");
        r.limit = Some(0);
        assert!(matches!(
            validate_request(&r, 20),
            Err(CrwError::InvalidRequest(_))
        ));
    }

    #[test]
    fn validate_accepts_basic_request() {
        assert!(validate_request(&req("rust async"), 20).is_ok());
    }

    #[test]
    fn map_search_error_timeout_to_timeout() {
        assert!(matches!(
            map_search_error(SearchError::Timeout, 7500),
            CrwError::Timeout(7500)
        ));
    }

    #[test]
    fn map_search_error_upstream_to_http_error() {
        let err = SearchError::Upstream {
            status: 503,
            body: "down".into(),
        };
        assert!(matches!(
            map_search_error(err, 5000),
            CrwError::HttpError(_)
        ));
    }

    #[test]
    fn _suppress_unused_search_source_warning() {
        let _ = SearchSource::Web;
    }
}
