//! Firecrawl-compatible Research API routes (`/v1/search/research/*`).
//!
//! Stateless primitives over live data (no self-hosted index): our OWN fastCRW
//! SearXNG search (web + research-mode, the primary recall driver) merged with
//! OpenAlex + Semantic Scholar via [`crw_search::research`]. The agent brain
//! (intent routing, exact-name reframing, leaderboard/survey) lives in the
//! research SKILL, not here — exactly like Firecrawl.
//!
//! Response shapes mirror Firecrawl's v2 research SDK ([`crw_core::research_types`])
//! so their SDK/CLI works drop-in against our base URL.

use axum::Json;
use axum::extract::{Path, Query, State};
use crw_core::error::CrwError;
use crw_core::research_types::{
    GithubResponse, PaperMetaResponse, PapersResponse, ReadPaperResponse, ResearchGithubItem,
    ResearchPassage, SimilarResponse,
};
use crw_search::research::{self, Mode, PaperHit, ResearchKeys, SearchFilters};
use crw_search::{SearxngClient, SearxngParams};
use serde::Deserialize;
use std::sync::Arc;

use crate::error::AppError;
use crate::state::AppState;

const DEFAULT_K: usize = 40;
const MAX_K: usize = 100;

fn keys(state: &AppState) -> ResearchKeys<'_> {
    ResearchKeys {
        openalex_key: state.config.search.openalex_api_key.as_deref(),
        openalex_mailto: state.config.search.openalex_mailto.as_deref(),
        s2_key: state.config.search.s2_api_key.as_deref(),
    }
}

fn searxng(state: &AppState) -> Result<Arc<SearxngClient>, CrwError> {
    state.searxng.as_ref().cloned().ok_or_else(|| {
        CrwError::SearchDisabled(
            "Search is disabled. Set [search].searxng_url or CRW_SEARCH__SEARXNG_URL.".into(),
        )
    })
}

fn clamp_k(k: Option<usize>) -> usize {
    k.unwrap_or(DEFAULT_K).clamp(1, MAX_K)
}

/// Join engines, or `None` when empty (sending `engines=` can silently empty
/// the SearXNG leg).
fn join_nonempty(v: &[String]) -> Option<String> {
    if v.is_empty() {
        None
    } else {
        Some(v.join(","))
    }
}

/// One SearXNG leg → arXiv-only [`PaperHit`]s. `engines: None` = plain web
/// (google/bing); `Some(joined)` = research-mode scholarly engines.
async fn searxng_papers(
    client: &SearxngClient,
    engines: Option<String>,
    query: &str,
) -> Vec<PaperHit> {
    let params = SearxngParams {
        q: query.to_string(),
        categories: None,
        language: Some("en".to_string()),
        time_range: None,
        engines,
        pageno: None,
        safesearch: None,
    };
    let Ok(resp) = client.fetch(&params).await else {
        return Vec::new();
    };
    resp.results
        .into_iter()
        .filter_map(|r| {
            let title = r.title.clone().unwrap_or_default();
            let blob = format!(
                "{} {} {}",
                r.url.unwrap_or_default(),
                title,
                r.content.unwrap_or_default()
            );
            PaperHit::from_searxng(&title, &blob, r.score.unwrap_or(0.0))
        })
        .collect()
}

#[derive(Deserialize)]
pub struct PapersQuery {
    query: String,
    k: Option<usize>,
    authors: Option<String>,
    categories: Option<String>,
    from: Option<String>,
    to: Option<String>,
}

/// `GET /v1/search/research/papers` — ranked paper search. Merges our own
/// fastCRW search (web + research-mode) with OpenAlex + SS, frequency-ranked
/// (the `research_tools.py` cascade core).
pub async fn search_papers(
    State(state): State<AppState>,
    Query(q): Query<PapersQuery>,
) -> Result<Json<PapersResponse>, AppError> {
    let client = searxng(&state)?;
    let k = clamp_k(q.k);
    let f = SearchFilters {
        authors: q.authors,
        categories: q.categories,
        from: q.from,
        to: q.to,
    };
    let kz = keys(&state);
    let research_engines = join_nonempty(&state.config.search.research_engines);
    // our own search (primary driver) + OpenAlex + SS, all in parallel
    let (web, scholar, oa_ss) = tokio::join!(
        searxng_papers(&client, None, &q.query),
        searxng_papers(&client, research_engines, &q.query),
        research::search_papers_pools(&kz, &q.query, k, &f),
    );
    let mut pools = vec![web, scholar];
    pools.extend(oa_ss);
    let results = research::merge_rank(pools, k);
    Ok(Json(PapersResponse {
        success: true,
        results,
    }))
}

#[derive(Deserialize)]
pub struct PaperQuery {
    query: Option<String>,
    k: Option<usize>,
}

/// `GET /v1/search/research/papers/{id}` — inspect metadata, or (with `?query`)
/// read top passages.
///
/// `ponytail:` read_passages is abstract-scoped (ranks abstract sentences by
/// query-term overlap). Full arXiv-body passages are the upgrade: scrape
/// `arxiv.org/html|pdf/<id>` via `state.renderer` + chunk + rank. Deferred —
/// abstract relevance carries the common "does this paper mention X" check, and
/// the heavy scrape plumbing (crw_crawl::single::scrape_url, 8 args) is a follow-up.
pub async fn get_paper(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<PaperQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let kz = keys(&state);
    let meta = research::inspect(&kz, &id)
        .await
        .ok_or_else(|| CrwError::NotFound(format!("paper not found: {id}")))?;
    match q.query {
        None => Ok(Json(
            serde_json::to_value(PaperMetaResponse {
                success: true,
                paper: meta,
            })
            .map_err(|e| CrwError::Internal(e.to_string()))?,
        )),
        Some(query) => {
            let k = q.k.unwrap_or(4).clamp(1, 20);
            let passages = rank_abstract_passages(meta.abstract_.as_deref(), &query, k);
            Ok(Json(
                serde_json::to_value(ReadPaperResponse {
                    success: true,
                    paper_id: meta.paper_id.clone(),
                    query,
                    passages,
                    paper: meta,
                })
                .map_err(|e| CrwError::Internal(e.to_string()))?,
            ))
        }
    }
}

/// Split an abstract into sentences, score each by query-term overlap, return
/// the top-k. Legal (abstract is CC0 metadata) and dependency-free.
fn rank_abstract_passages(abstract_: Option<&str>, query: &str, k: usize) -> Vec<ResearchPassage> {
    let Some(text) = abstract_ else {
        return Vec::new();
    };
    let qterms: Vec<String> = query
        .to_lowercase()
        .split_whitespace()
        .filter(|w| w.len() > 2)
        .map(String::from)
        .collect();
    let mut scored: Vec<ResearchPassage> = text
        .split(['.', '!', '?'])
        .map(str::trim)
        .filter(|s| s.len() > 20)
        .map(|sentence| {
            let low = sentence.to_lowercase();
            let hits = qterms.iter().filter(|t| low.contains(*t)).count();
            let score = if qterms.is_empty() {
                0.0
            } else {
                hits as f64 / qterms.len() as f64
            };
            ResearchPassage {
                text: sentence.to_string(),
                score,
            }
        })
        .collect();
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(k);
    scored
}

#[derive(Deserialize)]
pub struct SimilarQuery {
    intent: Option<String>,
    mode: Option<String>,
    k: Option<usize>,
}

/// `GET /v1/search/research/papers/{id}/similar` — citation-graph expansion.
pub async fn similar(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<SimilarQuery>,
) -> Result<Json<SimilarResponse>, AppError> {
    // Firecrawl requires `intent`; enforce for drop-in compat.
    if q.intent.as_deref().unwrap_or("").trim().is_empty() {
        return Err(CrwError::InvalidRequest("`intent` is required".into()).into());
    }
    let k = clamp_k(q.k);
    let mode = match q.mode.as_deref() {
        Some("citers") => Mode::Citers,
        Some("references") => Mode::References,
        _ => Mode::Similar,
    };
    let kz = keys(&state);
    let results = research::related(&kz, &id, mode, k).await;
    let pool_size = results.len();
    Ok(Json(SimilarResponse {
        success: true,
        results,
        pool_size,
        truncated: pool_size >= k,
        note: None,
    }))
}

#[derive(Deserialize)]
pub struct GithubQuery {
    query: String,
    k: Option<usize>,
}

/// `GET /v1/search/research/github` — GitHub search via our SearXNG github
/// engines. `ponytail:` SearXNG yields repo/readme hits, so `resultType` is
/// `repo_readme`; issue/PR/discussion granularity (Firecrawl's `github_history`)
/// is a follow-up if the github engines expose it.
pub async fn github(
    State(state): State<AppState>,
    Query(q): Query<GithubQuery>,
) -> Result<Json<GithubResponse>, AppError> {
    let client = searxng(&state)?;
    let k = q.k.unwrap_or(20).clamp(1, 100);
    let params = SearxngParams {
        q: q.query,
        categories: None,
        language: Some("en".to_string()),
        time_range: None,
        engines: join_nonempty(&state.config.search.github_engines),
        pageno: None,
        safesearch: None,
    };
    let resp = client
        .fetch(&params)
        .await
        .map_err(|e| CrwError::HttpError(format!("github search failed: {e}")))?;
    let results: Vec<ResearchGithubItem> = resp
        .results
        .into_iter()
        .take(k)
        .filter_map(|r| {
            let url = r.url?;
            // owner/name from a github URL path
            let repo = url
                .split("github.com/")
                .nth(1)
                .map(|p| p.split('/').take(2).collect::<Vec<_>>().join("/"))
                .unwrap_or_default();
            Some(ResearchGithubItem {
                result_type: "repo_readme".to_string(),
                repo,
                url,
                page_type: None,
                number: None,
                segment_count: None,
                readme_url: None,
                title: r.title.unwrap_or_default(),
                snippet: r.content.clone().unwrap_or_default(),
                content_md: r.content,
            })
        })
        .collect();
    Ok(Json(GithubResponse {
        success: true,
        results,
    }))
}
