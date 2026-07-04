use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use crw_core::Deadline;
use crw_core::config::LlmConfig;
use crw_core::error::CrwError;
use crw_core::types::{
    ApiResponse, LlmUsage, OutputFormat, ScrapeData, ScrapeRequest, SearchData, SearchRequest,
    SearchResponse, SearchResponseData, SearchResult, SearchScrapeOptions,
};
use crw_crawl::single::scrape_url;
use crw_extract::answer;
use crw_extract::summary;
use crw_search::{
    SearchError, SearxngClient, SearxngParams, SearxngResponse, map_to_searxng_params,
    transform_flat, transform_flat_reranked, transform_grouped,
};
use futures::stream::{self, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::task::JoinSet;

const DEFAULT_ANSWER_TOP_N: u32 = 5;
/// Default top-N for the calibrated answer path (feeds more sources so the
/// answer in result #6-8, or behind a failed top-5 scrape, still reaches the
/// model). Bounded by `MAX_ANSWER_TOP_N`.
const CALIBRATED_ANSWER_TOP_N: u32 = 8;
const MAX_ANSWER_TOP_N: u32 = 10;
/// Upper bound on query-expansion rewrites fetched + unioned per request, so a
/// request-supplied `query_expand_variants` can't fan out unbounded SearXNG
/// fetches. The extra pools are fetched concurrently in `fetch_expanded`.
const MAX_QUERY_EXPAND_VARIANTS: usize = 5;
/// Adaptive multi-round: max follow-up queries the evidence-scout issues in the
/// extra round, and how many results each contributes to the scrape pool.
/// Bounded so the single extra round stays well within the request deadline.
const MAX_SCOUT_QUERIES: usize = 2;
const SCOUT_FETCH_LIMIT: u32 = 6;
/// Minimum request-deadline budget that must remain before the adaptive
/// multi-round scout is allowed to start its extra round (scout LLM + up to
/// `MAX_SCOUT_QUERIES` fetches + scrapes + one re-synthesis). Below this, an
/// abstaining query returns round-1 immediately instead of risking a deadline
/// overrun (504) — this is what bounds the worst-case latency that enabling
/// `multi_round` adds.
const MULTI_ROUND_MIN_BUDGET_MS: u64 = 20_000;

/// Heuristic: did the synthesized answer ABSTAIN (sources lacked the fact)?
/// Aligned with `answer.rs`'s calibrated clause ("ONLY if the sources genuinely
/// do not contain the information, say so plainly"). Triggers the adaptive
/// multi-round scout. Conservative — only well-known abstention phrasings.
fn is_abstention(answer: &str) -> bool {
    let a = answer.to_lowercase();
    const MARKERS: &[&str] = &[
        "do not contain",
        "does not contain",
        "doesn't contain",
        "cannot answer",
        "can't answer",
        "cannot determine",
        "could not find",
        "couldn't find",
        "no information",
        "do not provide",
        "does not provide",
        "not mentioned in",
        "not specified",
        "unable to answer",
        "cannot be answered",
        "sources do not",
        "i cannot",
    ];
    MARKERS.iter().any(|m| a.contains(m))
}

/// Build a short evidence excerpt from the current candidate pool to brief the
/// evidence-scout (title + a markdown/snippet head per source, bounded).
fn evidence_excerpt(data: &SearchData, max_sources: usize, per_chars: usize) -> String {
    let pool: &Vec<SearchResult> = match data {
        SearchData::Flat(v) => v,
        SearchData::Grouped(g) => match g.web.as_ref() {
            Some(v) => v,
            None => return String::new(),
        },
    };
    let mut out = String::new();
    for r in pool.iter().take(max_sources) {
        let body = r.markdown.as_deref().unwrap_or(r.description.as_str());
        let snip: String = body.chars().take(per_chars).collect();
        out.push_str("- ");
        out.push_str(&r.title);
        out.push_str(" :: ");
        out.push_str(snip.trim());
        out.push('\n');
    }
    out
}

/// Merge freshly-scraped scout rows into the flat answer pool (dedup by URL,
/// only rows that actually carry markdown). Returns true if any were added.
/// Grouped data (the explicit-`sources` path) is left untouched — multi-round
/// targets the flat answer path. Recall-only: never removes existing sources.
fn merge_scraped(data: &mut SearchData, rows: Vec<SearchResult>) -> bool {
    if let SearchData::Flat(pool) = data {
        let mut seen: std::collections::HashSet<String> =
            pool.iter().map(|r| r.url.clone()).collect();
        let mut added = false;
        for r in rows {
            if r.markdown.is_some() && seen.insert(r.url.clone()) {
                pool.push(r);
                added = true;
            }
        }
        added
    } else {
        false
    }
}
const DEFAULT_MAX_CHARS_PER_SOURCE: usize = 8192;

/// Wave 4 (R2): hard cap on `max_tokens` per LLM leg (one summary call OR
/// the answer call). Independent of the user's configured `cfg.max_tokens`
/// because the SaaS-side `estimateMaxCreditCostForSearch` uses this number
/// to pre-reserve credits; a per-leg cap higher than this would let real
/// usage exceed the reservation. Mirror in
/// `crw-saas/src/lib/llm-pricing.ts::legCost` (default 1024).
const SEARCH_LLM_MAX_TOKENS_PER_LEG: u32 = 1024;

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

    // Request-deadline clock, started at handler entry. Used by the adaptive
    // multi-round gate (below) to decide whether enough budget remains to run a
    // second scout round without risking a 504.
    let req_deadline = Deadline::from_request_ms(state.config.effective_deadline_ms(None, None));

    // BYOK + LLM config is built up-front because multi-query expansion (below)
    // needs the LLM *before* the SearXNG fetch. Reused by the summarize/answer
    // legs further down. `llm_path` = this request enters LLM mode.
    let server_llm = state.config.extraction.llm.clone();
    let byok_llm = build_byok_search_llm_config(&req, server_llm.as_ref());
    let effective_llm = byok_llm.as_ref().or(server_llm.as_ref());
    let llm_path = req.answer.unwrap_or(false) || req.summarize_results.unwrap_or(false);

    let params = map_to_searxng_params(&req, &state.config.search);
    // Multi-query expansion (gated): on the LLM path, also fetch an
    // entity/keyword rewrite of the query and UNION the pools so the answer's
    // source is more likely to surface. Falls back to the single fetch.
    // Per-request override (eval A/B) wins over the server config; clamp so a
    // hostile caller can't fan out unbounded SearXNG fetches.
    let variants_n = req
        .query_expand_variants
        .unwrap_or(state.config.search.query_expand_variants)
        .clamp(1, MAX_QUERY_EXPAND_VARIANTS);
    // Phase C1: when expansion + scrapeOptions are both in play, overlap the
    // scrape of the original-query results with the expansion (LLM rewrite +
    // variant fetches) instead of doing them serially. The final pool is the
    // identical union, so the reranked source set is unchanged — only the
    // ~5-10s expansion overhead is hidden behind the original scrape.
    let c1_overlap = state.config.search.pipeline_overlap
        && state.config.search.query_expand
        && llm_path
        && req.scrape_options.is_some()
        && effective_llm.is_some();
    let mut prescraped: Vec<SearchResult> = Vec::new();
    let mut response = if c1_overlap {
        let llm = effective_llm.expect("c1_overlap requires effective_llm");
        let opts = req
            .scrape_options
            .as_ref()
            .expect("c1_overlap requires scrape_options");
        let orig = client
            .fetch(&params)
            .await
            .map_err(|e| map_search_error(e, state.config.search.timeout_ms, client.base_url()))?;
        let mut data_orig = SearchData::Flat(transform_flat_reranked(
            &orig,
            &req.query,
            limit,
            state.config.search.rerank_relevance,
        ));
        // Scrape the original results WHILE the expansion fetches run.
        let (_enr, variant_pools) = tokio::join!(
            enrich_with_scrape(&mut data_orig, opts, state),
            fetch_variant_pools(&client, &req.query, &params, llm, variants_n),
        );
        if let SearchData::Flat(v) = data_orig {
            prescraped = v;
        }
        let mut merged = orig;
        union_pools(&mut merged, variant_pools);
        merged
    } else if state.config.search.query_expand
        && llm_path
        && let Some(llm) = effective_llm
    {
        fetch_expanded(&client, &req.query, &params, llm, variants_n)
            .await
            .map_err(|e| map_search_error(e, state.config.search.timeout_ms, client.base_url()))?
    } else {
        client
            .fetch(&params)
            .await
            .map_err(|e| map_search_error(e, state.config.search.timeout_ms, client.base_url()))?
    };

    let has_sources = req.sources.as_ref().is_some_and(|s| !s.is_empty());
    // The LLM answer / summarize path feeds the top-N flat sources straight to
    // the model, so it must receive a clean, query-relevant pool. Re-rank the
    // flat pool on that path (unless disabled); the plain path keeps the raw
    // SaaS byte-parity `transform_flat` sort.
    let mut data = if has_sources {
        let sources = req.sources.clone().unwrap_or_default();
        SearchData::Grouped(transform_grouped(&response, &sources, limit))
    } else if llm_path && state.config.search.rerank_enabled {
        SearchData::Flat(transform_flat_reranked(
            &response,
            &req.query,
            limit,
            state.config.search.rerank_relevance,
        ))
    } else {
        SearchData::Flat(transform_flat(&response, limit))
    };

    // W0: parse SearXNG's infoboxes[]/answers[] (Wikidata/Wikipedia structured
    // facts the results[] transform discards) into pinned answer sources. Gated
    // default-off; empty when the flag is off or no structured data was returned.
    let mut structured_sources: Vec<answer::Source> = if state.config.search.use_structured_sources
    {
        crw_search::structured_facts(&response)
            .into_iter()
            .map(|f| {
                let md = f.to_markdown();
                (f.url, f.title, md)
            })
            .collect()
    } else {
        Vec::new()
    };

    // W3: deterministic Wikidata entity-relation lookup (gated, answer path).
    // For `<relation> of <entity>` queries the obscure-entity long tail web
    // search can't surface, resolve the fact via Wikidata and PIN it first.
    // 3s-bounded + cached; any miss/error leaves the normal path untouched.
    if state.config.search.wikidata_lookup
        && llm_path
        && let Some(f) = crw_search::wikidata::lookup(&req.query).await
    {
        let md = f.to_markdown();
        structured_sources.insert(0, (f.url, f.title, md));
    }

    // Page-2 fallback (gated, default-off): when the reranked clean pool came
    // back thinner than the answer needs (junk filter stripped a sparse first
    // page), fetch the SAME query's page 2 ONCE, union it in (dedup by URL like
    // `fetch_expanded`), and re-rank. Trigger is evaluated POST-rerank so a
    // junk-heavy first page doesn't suppress it; extra load only fires on
    // already-under-yielding queries. Recall-only — synthesis/abstention in
    // `answer.rs` is untouched.
    if state.config.search.page2_fallback
        && llm_path
        && state.config.search.rerank_enabled
        && !has_sources
    {
        let top_n = req
            .answer_top_n
            .unwrap_or(DEFAULT_ANSWER_TOP_N)
            .min(MAX_ANSWER_TOP_N) as usize;
        let clean_count = match &data {
            SearchData::Flat(v) => v.len(),
            SearchData::Grouped(_) => top_n,
        };
        if clean_count < top_n {
            let mut p2 = params.clone();
            p2.pageno = Some(2);
            if let Ok(resp2) = client.fetch(&p2).await {
                let mut seen: std::collections::HashSet<String> = response
                    .results
                    .iter()
                    .filter_map(|r| r.url.clone())
                    .collect();
                for row in resp2.results {
                    if let Some(u) = row.url.clone()
                        && seen.insert(u)
                    {
                        response.results.push(row);
                    }
                }
                response.number_of_results = response.results.len() as u64;
                data = SearchData::Flat(transform_flat_reranked(
                    &response,
                    &req.query,
                    limit,
                    state.config.search.rerank_relevance,
                ));
            }
        }
    }

    // Phase C1: fold the original-results scrapes done during the overlap back
    // into the final (reranked-over-union) source set. Entries that match by URL
    // get their scraped fields reused; enrich_with_scrape then skips them
    // (metadata.is_some()) and only scrapes the URLs the expansion newly added.
    if !prescraped.is_empty()
        && let SearchData::Flat(v) = &mut data
    {
        let by_url: std::collections::HashMap<&str, &SearchResult> =
            prescraped.iter().map(|r| (r.url.as_str(), r)).collect();
        for r in v.iter_mut() {
            if r.metadata.is_none()
                && let Some(src) = by_url.get(r.url.as_str())
                && src.metadata.is_some()
            {
                r.markdown = src.markdown.clone();
                r.html = src.html.clone();
                r.raw_html = src.raw_html.clone();
                r.links = src.links.clone();
                r.metadata = src.metadata.clone();
            }
        }
    }

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

    // `effective_llm` / `byok_llm` / `server_llm` were built up-front (above).
    let wants_summaries = req.summarize_results.unwrap_or(false);
    let wants_answer = req.answer.unwrap_or(false);
    // Wave 4 (R1): once we enter LLM mode the response MUST carry a
    // non-null llmUsage object (the always-present invariant the SaaS
    // 5-branch dispatch relies on). We aggregate summary + answer counts
    // into this builder and emit it at every return path below.
    let mut llm_attempted = false;
    let mut agg_input_tokens: u32 = 0;
    let mut agg_output_tokens: u32 = 0;
    let mut agg_cache_hit: u32 = 0;
    let mut agg_cache_miss: u32 = 0;
    let mut agg_calls: u32 = 0;
    let mut agg_executed_summaries: u32 = 0;
    let mut agg_answer_executed = false;
    let mut agg_provider: String = String::new();
    let mut agg_model: String = String::new();
    let mut agg_truncated = false;
    let merge_usage = |agg_input_tokens: &mut u32,
                       agg_output_tokens: &mut u32,
                       agg_cache_hit: &mut u32,
                       agg_cache_miss: &mut u32,
                       agg_calls: &mut u32,
                       agg_provider: &mut String,
                       agg_model: &mut String,
                       agg_truncated: &mut bool,
                       u: &LlmUsage| {
        *agg_input_tokens = agg_input_tokens.saturating_add(u.input_tokens);
        *agg_output_tokens = agg_output_tokens.saturating_add(u.output_tokens);
        *agg_cache_hit = agg_cache_hit.saturating_add(u.cache_hit_input_tokens.unwrap_or(0));
        *agg_cache_miss = agg_cache_miss.saturating_add(u.cache_miss_input_tokens.unwrap_or(0));
        *agg_calls = agg_calls.saturating_add(u.calls.max(1));
        if agg_provider.is_empty() {
            *agg_provider = u.provider.clone();
        }
        if agg_model.is_empty() {
            *agg_model = u.model.clone();
        }
        *agg_truncated = *agg_truncated || u.truncated;
    };

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
                llm_attempted = true;
                // Wave 4 (R2): cap max_tokens at SEARCH_LLM_MAX_TOKENS_PER_LEG so
                // a single leg can never exceed the SaaS pre-reservation in
                // estimateMaxCreditCostForSearch.
                let mut leg_cfg = llm.clone();
                leg_cfg.max_tokens = leg_cfg.max_tokens.min(SEARCH_LLM_MAX_TOKENS_PER_LEG);
                // Eval determinism: a request-supplied answer temperature (the
                // benchmark harness sets 0) overrides the provider default so
                // A/B runs are reproducible. None = current prod behavior.
                if req.answer_temperature.is_some() {
                    leg_cfg.temperature = req.answer_temperature;
                }
                if wants_summaries {
                    let (count, usages) = attach_result_summaries(
                        &mut data,
                        &leg_cfg,
                        leg_cfg.max_concurrency,
                        req.summary_prompt.as_deref(),
                        req.max_content_chars,
                    )
                    .await;
                    agg_executed_summaries = count.ok as u32;
                    for u in usages.into_iter().flatten() {
                        merge_usage(
                            &mut agg_input_tokens,
                            &mut agg_output_tokens,
                            &mut agg_cache_hit,
                            &mut agg_cache_miss,
                            &mut agg_calls,
                            &mut agg_provider,
                            &mut agg_model,
                            &mut agg_truncated,
                            &u,
                        );
                    }
                    if count.failed > 0 {
                        warnings.push(format!(
                            "{} of {} per-result summaries failed",
                            count.failed,
                            count.failed + count.ok
                        ));
                    }
                }
                if wants_answer {
                    // List-format answer (gated): a request override wins, else
                    // the server flag; either way it only fires on list-intent
                    // queries ("best/top X in Y"). Factual queries keep prose.
                    let list_format = req
                        .answer_list_format
                        .unwrap_or(state.config.search.answer_list_format)
                        && answer::is_list_intent(&req.query);
                    match synthesize_answer(
                        &req,
                        &data,
                        &leg_cfg,
                        state.config.search.passage_select,
                        state.config.search.answer_calibrated,
                        state.config.search.snippet_fallback,
                        state.config.search.answer_guarded,
                        &structured_sources,
                        list_format,
                    )
                    .await
                    {
                        Ok((mut ans, mut cites, ans_usage, mut ans_warns)) => {
                            warnings.append(&mut ans_warns);
                            agg_answer_executed = true;
                            if let Some(ref u) = ans_usage {
                                merge_usage(
                                    &mut agg_input_tokens,
                                    &mut agg_output_tokens,
                                    &mut agg_cache_hit,
                                    &mut agg_cache_miss,
                                    &mut agg_calls,
                                    &mut agg_provider,
                                    &mut agg_model,
                                    &mut agg_truncated,
                                    u,
                                );
                            }
                            // Adaptive multi-round (gated): if round-1 ABSTAINED,
                            // the evidence-scout issues targeted follow-ups; we
                            // scrape them, union into the pool, and re-synthesize
                            // ONCE. Recall-only — a still-abstaining round-2 is
                            // discarded (keep round-1). Only fires on abstention,
                            // so the single-shot fast path is unchanged for hits.
                            let want_multi =
                                req.multi_round.unwrap_or(state.config.search.multi_round);
                            // Deadline budget: the extra scout round can add tens
                            // of seconds. If too little of the request deadline
                            // remains, skip it and return round-1 promptly rather
                            // than risk a 504 — this caps the worst-case latency
                            // multi_round would otherwise add.
                            let multi_budget_ok = req_deadline.remaining().as_millis() as u64
                                >= MULTI_ROUND_MIN_BUDGET_MS;
                            if want_multi && is_abstention(&ans) && !multi_budget_ok {
                                warnings.push(
                                    "multi-round skipped: insufficient deadline budget remaining"
                                        .to_string(),
                                );
                            }
                            if want_multi
                                && is_abstention(&ans)
                                && multi_budget_ok
                                && let Some(opts) = req.scrape_options.as_ref()
                            {
                                let evidence = evidence_excerpt(&data, 5, 400);
                                let scout_qs = crw_extract::llm::scout_followups(
                                    &leg_cfg,
                                    &req.query,
                                    &evidence,
                                    MAX_SCOUT_QUERIES,
                                )
                                .await;
                                let mut grew = false;
                                for sq in scout_qs {
                                    let mut sp = params.clone();
                                    sp.q = sq;
                                    if let Ok(resp2) = client.fetch(&sp).await {
                                        let extra = transform_flat_reranked(
                                            &resp2,
                                            &req.query,
                                            SCOUT_FETCH_LIMIT,
                                            state.config.search.rerank_relevance,
                                        );
                                        let mut sd = SearchData::Flat(extra);
                                        let _ = enrich_with_scrape(&mut sd, opts, state).await;
                                        if let SearchData::Flat(rows) = sd {
                                            grew |= merge_scraped(&mut data, rows);
                                        }
                                    }
                                }
                                if grew
                                    && let Ok((ans2, cites2, usage2, mut warns2)) =
                                        synthesize_answer(
                                            &req,
                                            &data,
                                            &leg_cfg,
                                            state.config.search.passage_select,
                                            state.config.search.answer_calibrated,
                                            state.config.search.snippet_fallback,
                                            state.config.search.answer_guarded,
                                            &structured_sources,
                                            list_format,
                                        )
                                        .await
                                {
                                    // Round-2 actually called the LLM and consumed
                                    // tokens, so its usage MUST be merged for honest
                                    // accounting even if it abstained — only the
                                    // ANSWER/citations are adopted conditionally
                                    // (when round-2 produced a non-abstaining
                                    // answer). Merging usage before the abstention
                                    // check prevents silently dropping the round-2
                                    // token cost.
                                    if let Some(ref u) = usage2 {
                                        merge_usage(
                                            &mut agg_input_tokens,
                                            &mut agg_output_tokens,
                                            &mut agg_cache_hit,
                                            &mut agg_cache_miss,
                                            &mut agg_calls,
                                            &mut agg_provider,
                                            &mut agg_model,
                                            &mut agg_truncated,
                                            u,
                                        );
                                    }
                                    if !is_abstention(&ans2) {
                                        ans = ans2;
                                        cites = cites2;
                                        warnings.append(&mut warns2);
                                    }
                                }
                            }
                            let aggregated = build_aggregated_usage(
                                agg_input_tokens,
                                agg_output_tokens,
                                agg_cache_hit,
                                agg_cache_miss,
                                agg_calls,
                                agg_executed_summaries,
                                agg_answer_executed,
                                agg_provider.clone(),
                                agg_model.clone(),
                                agg_truncated,
                                &leg_cfg,
                            );
                            let wrapped = SearchResponseData {
                                results: data,
                                answer: Some(ans),
                                citations: cites,
                                llm_usage: Some(aggregated),
                                warnings,
                            };
                            let mut resp = ApiResponse::ok(wrapped);
                            resp.warning = warning;
                            return Ok(resp);
                        }
                        Err(msg) => {
                            // Log the raw upstream error server-side, but never
                            // surface it to the client: `{msg}` can carry the
                            // managed-LLM provider name + raw HTTP status (P1-2).
                            tracing::warn!(error = %msg, "answer synthesis failed");
                            warnings.push("answer synthesis unavailable".to_string());
                        }
                    }
                }
            }
        }
    }

    // R1 always-present invariant: if we attempted LLM work, emit the
    // aggregated usage even when zero tokens were consumed (e.g. all
    // summaries failed and no answer leg ran). The SaaS dispatch maps
    // (executedSummaries == 0 && answerExecuted == false && tokens == 0)
    // to Branch 1 (no-op refund); anything else routes correctly.
    let final_usage = if llm_attempted {
        Some(build_aggregated_usage(
            agg_input_tokens,
            agg_output_tokens,
            agg_cache_hit,
            agg_cache_miss,
            agg_calls,
            agg_executed_summaries,
            agg_answer_executed,
            if agg_provider.is_empty() {
                effective_llm
                    .map(|c| c.provider.clone())
                    .unwrap_or_default()
            } else {
                agg_provider
            },
            if agg_model.is_empty() {
                effective_llm.map(|c| c.model.clone()).unwrap_or_default()
            } else {
                agg_model
            },
            agg_truncated,
            effective_llm
                .map(|c| {
                    let mut c = c.clone();
                    c.max_tokens = c.max_tokens.min(SEARCH_LLM_MAX_TOKENS_PER_LEG);
                    c
                })
                .as_ref()
                .unwrap_or(&crw_core::config::LlmConfig::default()),
        ))
    } else {
        None
    };

    let wrapped = SearchResponseData {
        results: data,
        answer: None,
        citations: Vec::new(),
        llm_usage: final_usage,
        warnings,
    };
    let mut resp = ApiResponse::ok(wrapped);
    resp.warning = warning;
    Ok(resp)
}

#[allow(clippy::too_many_arguments)]
fn build_aggregated_usage(
    input_tokens: u32,
    output_tokens: u32,
    cache_hit: u32,
    cache_miss: u32,
    calls: u32,
    executed_summaries: u32,
    answer_executed: bool,
    provider: String,
    model: String,
    truncated: bool,
    fallback_cfg: &crw_core::config::LlmConfig,
) -> LlmUsage {
    LlmUsage {
        input_tokens,
        output_tokens,
        total_tokens: input_tokens.saturating_add(output_tokens),
        estimated_cost_usd: None,
        model: if model.is_empty() {
            fallback_cfg.model.clone()
        } else {
            model
        },
        provider: if provider.is_empty() {
            fallback_cfg.provider.clone()
        } else {
            provider
        },
        cache_hit_input_tokens: if cache_hit == 0 {
            None
        } else {
            Some(cache_hit)
        },
        cache_miss_input_tokens: if cache_miss == 0 {
            None
        } else {
            Some(cache_miss)
        },
        truncated,
        calls: calls.max(1),
        executed_summaries,
        answer_executed,
    }
}

#[derive(Default)]
struct SummaryFanoutCount {
    ok: usize,
    failed: usize,
}

/// Fan-out summary calls across all results that have markdown. Bounded by
/// `max_concurrency`. Pattern mirrors `crates/crw-crawl/src/sitemap.rs`.
///
/// Wave 4 (R1): returns the per-call `Option<LlmUsage>` for every job
/// alongside the ok/failed count so the caller can aggregate token totals
/// across summaries + answer.
async fn attach_result_summaries(
    data: &mut SearchData,
    cfg: &LlmConfig,
    max_concurrency: usize,
    user_prompt: Option<&str>,
    max_content_chars: Option<usize>,
) -> (SummaryFanoutCount, Vec<Option<LlmUsage>>) {
    let targets: &mut Vec<SearchResult> = match data {
        SearchData::Flat(v) => v,
        SearchData::Grouped(g) => match g.web.as_mut() {
            Some(v) if !v.is_empty() => v,
            _ => return (SummaryFanoutCount::default(), Vec::new()),
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
        return (SummaryFanoutCount::default(), Vec::new());
    }
    let cfg_owned = cfg.clone();
    let user_prompt_owned: Option<String> = user_prompt.map(str::to_owned);
    let concurrency = max_concurrency.max(1);
    type SummaryOutcome = (usize, Result<(String, Option<LlmUsage>), String>);
    let results: Vec<SummaryOutcome> = stream::iter(jobs)
        .map(|(idx, md)| {
            let cfg = cfg_owned.clone();
            let user_prompt = user_prompt_owned.clone();
            async move {
                let outcome =
                    summary::summarize(&md, &cfg, user_prompt.as_deref(), max_content_chars)
                        .await
                        .map(|r| (r.content, r.usage))
                        .map_err(|e| e.to_string());
                (idx, outcome)
            }
        })
        .buffer_unordered(concurrency)
        .collect()
        .await;

    let mut count = SummaryFanoutCount::default();
    let mut usages: Vec<Option<LlmUsage>> = Vec::with_capacity(results.len());
    for (idx, res) in results {
        match res {
            Ok((text, usage)) => {
                if let Some(slot) = targets.get_mut(idx) {
                    slot.summary = Some(text);
                    count.ok += 1;
                    usages.push(usage);
                }
            }
            Err(_) => {
                count.failed += 1;
                usages.push(None);
            }
        }
    }
    (count, usages)
}

#[allow(clippy::too_many_arguments)]
async fn synthesize_answer(
    req: &SearchRequest,
    data: &SearchData,
    cfg: &LlmConfig,
    passage_select: bool,
    calibrated: bool,
    snippet_fallback: bool,
    guarded: bool,
    // W0: structured facts (infoboxes/answers) to PIN at the front of the pool.
    structured: &[answer::Source],
    list_format: bool,
) -> Result<
    (
        String,
        Vec<crw_core::types::Citation>,
        Option<crw_core::types::LlmUsage>,
        Vec<String>,
    ),
    String,
> {
    // Calibrated answer feeds more sources by default (Pattern A: the answer
    // often sits in result #6-8, or a failed top-5 scrape thinned the pool) and
    // uses the anti-hedge prompt clause. An explicit request `answer_top_n`
    // still wins. Capped by MAX_ANSWER_TOP_N.
    let default_top_n = if calibrated {
        CALIBRATED_ANSWER_TOP_N
    } else {
        DEFAULT_ANSWER_TOP_N
    };
    let top_n = req
        .answer_top_n
        .unwrap_or(default_top_n)
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
    let scraped: Vec<answer::Source> = pool
        .iter()
        .filter_map(|r| {
            if let Some(md) = r.markdown.as_ref() {
                Some((r.url.clone(), r.title.clone(), md.clone()))
            } else if snippet_fallback {
                // Scrape failed/empty — instead of dropping the result (which
                // can lose the answer-bearing page, Pattern A), fall back to the
                // SearXNG snippet. It's verbatim upstream text, so it can only
                // surface a fact already present, never invent one.
                let desc = r.description.trim();
                if desc.is_empty() {
                    None
                } else {
                    Some((r.url.clone(), r.title.clone(), format!("[snippet] {desc}")))
                }
            } else {
                None
            }
        })
        .take(top_n)
        .collect();
    // W0: PIN structured facts (Wikidata/Wikipedia infobox/answers) at the front
    // so the synthesizer sees them first. They are still UNTRUSTED-wrapped by
    // `answer::synthesize` — this widens evidence, it does not bypass safety.
    let sources: Vec<answer::Source> = if structured.is_empty() {
        scraped
    } else {
        structured.iter().cloned().chain(scraped).collect()
    };
    if sources.is_empty() {
        return Err("no results carry markdown to synthesize an answer from".into());
    }
    // Passage-select reduces each large source to its query-relevant passages
    // before synthesis (monotone-safe: falls back to the full source on any
    // failure). Gated; off = byte-identical to plain synthesize.
    let result = if passage_select {
        answer::synthesize_selected(
            &req.query,
            &sources,
            cfg,
            cap,
            req.answer_prompt.as_deref(),
            calibrated,
            guarded,
            list_format,
        )
        .await
    } else {
        answer::synthesize(
            &req.query,
            &sources,
            cfg,
            cap,
            req.answer_prompt.as_deref(),
            calibrated,
            guarded,
            list_format,
        )
        .await
    }
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
    // Never inherit the server's reasoning_effort into a BYOK request — the
    // customer's endpoint must receive only what they explicitly configure.
    cfg.reasoning_effort = None;
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

/// Map a transport/timeout/upstream `SearchError` onto the HTTP `CrwError`.
/// `base_url` is the configured SearXNG URL; the transport (`target_unreachable`)
/// arm names its **origin** (issue #90) so the operator sees *which* host failed
/// — sanitized, so a credentialed URL never reaches the response. Timeouts keep
/// `error_code: "timeout"`; the host is correlated via the startup log instead.
fn map_search_error(err: SearchError, timeout_ms: u64, base_url: &str) -> CrwError {
    match err {
        SearchError::Timeout => CrwError::Timeout(timeout_ms),
        SearchError::Upstream { status, body } => CrwError::HttpError(format!(
            "SearXNG returned HTTP {status}: {}",
            body.chars().take(200).collect::<String>()
        )),
        SearchError::InvalidResponse(msg) => {
            CrwError::HttpError(format!("SearXNG returned invalid JSON: {msg}"))
        }
        SearchError::Transport(msg) => CrwError::TargetUnreachable(format!(
            "SearXNG ({}): {msg}",
            crate::diagnostics::sanitize_url_origin(base_url)
        )),
    }
}

/// Multi-query expansion: fetch the original query plus an LLM-generated
/// entity/keyword rewrite, then UNION the candidate pools (dedupe by URL,
/// original results kept first so they retain priority). Recall can only
/// increase vs a single fetch. The original fetch's error propagates (same
/// failure semantics as the single-fetch path); a failed variant fetch is
/// ignored. If the rewrite is empty/trivial, this is exactly the single fetch.
/// Expand the query (LLM rewrite) and fetch all variant pools concurrently
/// (bounded by the variant count) so N rewrites cost ~one extra fetch of
/// wall-clock, not N sequential ones. Does NOT fetch the original query — the
/// caller owns that, which lets the C1 overlap path scrape the original results
/// while this runs. A failed variant fetch is dropped (recall-only, never fatal).
async fn fetch_variant_pools(
    client: &SearxngClient,
    query: &str,
    base_params: &SearxngParams,
    llm: &LlmConfig,
    max_variants: usize,
) -> Vec<SearxngResponse> {
    let mut leg = llm.clone();
    leg.max_tokens = leg.max_tokens.min(SEARCH_LLM_MAX_TOKENS_PER_LEG);
    let variants = crw_extract::llm::expand_query(&leg, query, max_variants).await;
    if variants.is_empty() {
        return Vec::new();
    }
    stream::iter(variants)
        .map(|v| {
            let client = client.clone();
            let mut vp = base_params.clone();
            vp.q = v;
            async move { client.fetch(&vp).await.ok() }
        })
        .buffer_unordered(max_variants.max(1))
        .filter_map(|r| async move { r })
        .collect()
        .await
}

/// Union variant pools into `merged`, deduping by URL (recall-only — never
/// removes existing sources). Shared by the serial and C1-overlap paths so both
/// produce the identical unioned pool.
fn union_pools(merged: &mut SearxngResponse, pools: Vec<SearxngResponse>) {
    let mut seen: std::collections::HashSet<String> = merged
        .results
        .iter()
        .filter_map(|r| r.url.clone())
        .collect();
    for resp in pools {
        for row in resp.results {
            if let Some(u) = row.url.clone()
                && seen.insert(u)
            {
                merged.results.push(row);
            }
        }
    }
    merged.number_of_results = merged.results.len() as u64;
}

async fn fetch_expanded(
    client: &SearxngClient,
    query: &str,
    base_params: &SearxngParams,
    llm: &LlmConfig,
    max_variants: usize,
) -> Result<SearxngResponse, SearchError> {
    // Original fetch overlaps the expansion+variant fetches; union is identical.
    let (orig, variant_pools) = tokio::join!(
        client.fetch(base_params),
        fetch_variant_pools(client, query, base_params, llm, max_variants)
    );
    let mut merged = orig?;
    union_pools(&mut merged, variant_pools);
    Ok(merged)
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
        // C1 overlap: a slot already enriched by the original-results prefetch
        // (metadata set by apply_scrape_to_result) is reused, not re-scraped.
        if r.metadata.is_some() {
            continue;
        }
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
    let country = opts.country.clone();
    let semaphore = Arc::new(tokio::sync::Semaphore::new(
        state.config.crawler.max_concurrency.max(1),
    ));
    let mut set: JoinSet<(usize, Result<ScrapeData, String>)> = JoinSet::new();

    for (idx, url) in jobs {
        let formats = formats.clone();
        let country = country.clone();
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
                proxy_list: Vec::new(),
                proxy_rotation: None,
                country,
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
                change_tracking: None,
                goal: None,
                judge_enabled: None,
                parsers: None,
                screenshot_full_page: false,
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
                // P3-4: mark the result so a partial scrape is observable to the
                // caller instead of looking identical to "no markdown found".
                slot.error = Some(msg);
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
            answer_temperature: None,
            query_expand_variants: None,
            multi_round: None,
            answer_list_format: None,
            max_content_chars: None,
        }
    }

    #[test]
    fn is_abstention_detects_marker_phrases() {
        assert!(is_abstention(
            "The sources do not contain this information."
        ));
        assert!(is_abstention("I cannot answer that from the sources."));
        assert!(is_abstention("That is not mentioned in the sources."));
        assert!(is_abstention("The provided sources do not provide a year."));
        assert!(is_abstention("I could not find the answer."));
        assert!(is_abstention("The date is not specified anywhere."));
    }

    #[test]
    fn is_abstention_false_for_normal_answer() {
        assert!(!is_abstention("The capital of Serbia is Belgrade."));
        assert!(!is_abstention("Rust was first released publicly in 2010."));
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
            map_search_error(SearchError::Timeout, 7500, "http://searxng:8080"),
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
            map_search_error(err, 5000, "http://searxng:8080"),
            CrwError::HttpError(_)
        ));
    }

    #[test]
    fn map_search_error_transport_names_sanitized_host() {
        // issue #90: the unreachable error must name the configured host so the
        // operator knows *what* failed — but origin-only, never the raw URL.
        let err = SearchError::Transport("dns error: failed to lookup address".into());
        let mapped = map_search_error(err, 5000, "https://user:pass@searxng:8080/tok?k=v");
        match mapped {
            CrwError::TargetUnreachable(msg) => {
                assert!(msg.contains("https://searxng:8080"), "{msg}");
                assert!(!msg.contains("user"), "must not leak userinfo: {msg}");
                assert!(!msg.contains("pass"), "must not leak credentials: {msg}");
                assert!(!msg.contains("tok"), "must not leak path token: {msg}");
            }
            other => panic!("expected TargetUnreachable, got {other:?}"),
        }
    }

    #[test]
    fn byok_config_clears_reasoning_effort() {
        // A BYOK request must never inherit the server's reasoning_effort.
        let server_cfg = LlmConfig {
            reasoning_effort: Some("none".into()),
            ..Default::default()
        };
        let mut r = req("hello");
        r.llm_api_key = Some("byok-key".into());
        let byok = build_byok_search_llm_config(&r, Some(&server_cfg))
            .expect("byok config built when llm_api_key present");
        assert_eq!(byok.reasoning_effort, None);
        assert_eq!(byok.api_key, "byok-key");
    }

    #[test]
    fn byok_config_none_without_api_key() {
        let server_cfg = LlmConfig {
            reasoning_effort: Some("none".into()),
            ..Default::default()
        };
        // No llm_api_key => not a BYOK request.
        let byok = build_byok_search_llm_config(&req("hello"), Some(&server_cfg));
        assert!(byok.is_none());
    }

    #[test]
    fn _suppress_unused_search_source_warning() {
        let _ = SearchSource::Web;
    }
}
