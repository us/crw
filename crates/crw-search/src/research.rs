//! Live research data layer for the `/v1/search/research/*` endpoints.
//!
//! Ports the proven `arxivqa-bench/research_tools.py` cascade (which scored
//! 59.6% recall on ArXivQA, beating Firecrawl's 53.3%) to Rust: OpenAlex
//! `/works` search + Semantic Scholar `/paper/search` + `/snippet/search`
//! booster, and the citation graph (SS references/citations/recommendations
//! with OpenAlex fallback). NO self-hosted index — all live.
//!
//! This crate owns only the OpenAlex + SS HTTP legs. The OWN fastCRW SearXNG
//! search leg (the primary recall driver) and any arXiv PDF scrape live in the
//! route handler (`crw-server`, which has `state.searxng` + `state.renderer`),
//! which merges its hits into [`merge_rank`]. Keys are passed per-call from the
//! route's `AppConfig` (this module holds only stateless infra: client, cache,
//! semaphore).
//!
//! Etiquette: dedicated client + descriptive UA, per-source concurrency cap,
//! 24h cache, exponential backoff on 429/5xx (OpenAlex ~10 rps; SS 1 rps shared
//! key — SS is a BOOSTER, its failures degrade gracefully to OpenAlex + SearXNG).

use crw_core::research_types::{ResearchPaperMeta, ResearchPaperResult};
use moka::future::Cache;
use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::Semaphore;

const UA: &str = "crw-opencore/0.x (https://fastcrw.com; contact@fastcrw.com) reqwest";
const TIMEOUT: Duration = Duration::from_secs(20);
const MAX_CONCURRENCY: usize = 8;
const CACHE_TTL: Duration = Duration::from_secs(24 * 3600);
// Entries hold full OpenAlex/SS JSON responses (~10-50KB each). 20k entries
// could pin 200MB-1GB and OOM-restart crw-api under research load; 3k keeps the
// cache useful while bounding it to ~50-150MB. ponytail: cap, not a byte-weigher.
const CACHE_CAP: u64 = 3_000;

/// Per-call credentials, borrowed from the route's `AppConfig`.
#[derive(Clone, Copy, Default)]
pub struct ResearchKeys<'a> {
    pub openalex_key: Option<&'a str>,
    pub openalex_mailto: Option<&'a str>,
    pub s2_key: Option<&'a str>,
}

/// OpenAlex `/works` filters (all optional, AND-combined). Maps the Firecrawl
/// `authors`/`categories`/`from`/`to` query params.
#[derive(Clone, Default)]
pub struct SearchFilters {
    pub authors: Option<String>,
    pub categories: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}

/// Citation-graph expansion mode for `/papers/{id}/similar`.
#[derive(Clone, Copy)]
pub enum Mode {
    Similar,
    Citers,
    References,
}

struct Infra {
    http: reqwest::Client,
    cache: Cache<String, serde_json::Value>,
}

fn infra() -> Option<&'static Infra> {
    static I: OnceLock<Option<Infra>> = OnceLock::new();
    I.get_or_init(|| {
        let http = reqwest::Client::builder()
            .user_agent(UA)
            .timeout(TIMEOUT)
            .build()
            .ok()?;
        Some(Infra {
            http,
            cache: Cache::builder()
                .max_capacity(CACHE_CAP)
                .time_to_live(CACHE_TTL)
                .build(),
        })
    })
    .as_ref()
}

fn sem() -> &'static Semaphore {
    static S: OnceLock<Semaphore> = OnceLock::new();
    S.get_or_init(|| Semaphore::new(MAX_CONCURRENCY))
}

fn arxiv_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\d{4}\.\d{4,5}").unwrap())
}

fn ver_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)v\d+$").unwrap())
}

/// URL-encode a query-string value (via the `url` crate, no extra dep).
fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// Normalize an arXiv id: strip a leading `arXiv:`/`arxiv:` prefix and a trailing
/// version (`arXiv:2105.05233v3` -> `2105.05233`), lowercase. Matches
/// `research_tools.py`'s `re.sub(r"v\d+$", "", id)` (NOT a split on 'v', which
/// mangles the `arxiv:` prefix).
fn norm_arxiv(s: &str) -> String {
    let s = s.trim();
    let s = s
        .strip_prefix("arXiv:")
        .or_else(|| s.strip_prefix("arxiv:"))
        .unwrap_or(s);
    ver_re().replace(s, "").to_lowercase()
}

/// Cached GET → JSON with exponential backoff on 429/5xx (2s, 4s, 8s).
/// `x_api_key` adds the SS `x-api-key` header.
async fn get_json(url: &str, x_api_key: Option<&str>) -> Option<serde_json::Value> {
    let inf = infra()?;
    let ck = format!("{}|{}", x_api_key.unwrap_or(""), url);
    if let Some(hit) = inf.cache.get(&ck).await {
        return Some(hit);
    }
    let _permit = sem().acquire().await.ok()?;
    for i in 0..4u32 {
        let mut req = inf.http.get(url);
        if let Some(k) = x_api_key {
            req = req.header("x-api-key", k);
        }
        match req.send().await {
            Ok(r) if r.status().is_success() => {
                if let Ok(v) = r.json::<serde_json::Value>().await {
                    inf.cache.insert(ck, v.clone()).await;
                    return Some(v);
                }
                return None;
            }
            Ok(r) if r.status().as_u16() == 429 || r.status().is_server_error() => {
                if i == 3 {
                    return None;
                }
                tokio::time::sleep(Duration::from_secs(2u64 << i)).await;
            }
            _ => return None, // other 4xx / network error -> give up (no point retrying)
        }
    }
    None
}

/// Reconstruct plaintext from OpenAlex's `abstract_inverted_index`
/// (`{word: [positions...]}`) → ordered words joined by spaces.
fn reconstruct_abstract(inv: &serde_json::Value) -> Option<String> {
    let obj = inv.as_object()?;
    let mut pairs: Vec<(u64, &str)> = Vec::new();
    for (word, positions) in obj {
        if let Some(arr) = positions.as_array() {
            for p in arr {
                if let Some(pos) = p.as_u64() {
                    pairs.push((pos, word.as_str()));
                }
            }
        }
    }
    if pairs.is_empty() {
        return None;
    }
    pairs.sort_by_key(|(p, _)| *p);
    Some(
        pairs
            .into_iter()
            .map(|(_, w)| w)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Internal candidate before merge/rank.
#[derive(Clone)]
pub struct PaperHit {
    pub work_id: Option<String>,
    pub arxiv: Option<String>,
    pub doi: Option<String>,
    pub title: String,
    pub abstract_: Option<String>,
    pub cited_by: u64,
    pub score: f64,
}

impl PaperHit {
    /// Dedup key: arXiv id wins, then DOI, then lowercased title.
    fn key(&self) -> String {
        if let Some(a) = &self.arxiv {
            return format!("arxiv:{a}");
        }
        if let Some(d) = &self.doi {
            return format!("doi:{}", d.to_lowercase());
        }
        format!("title:{}", self.title.to_lowercase())
    }

    /// Build a minimal hit from a SearXNG result (route passes these in). The
    /// arXiv id is regex-extracted from the url/title/content.
    pub fn from_searxng(title: &str, blob: &str, score: f64) -> Option<Self> {
        let arxiv = arxiv_re().find(blob).map(|m| norm_arxiv(m.as_str()));
        arxiv.as_ref()?; // only keep scholarly (arXiv) hits from the web leg
        Some(PaperHit {
            work_id: None,
            arxiv,
            doi: None,
            title: title.to_string(),
            abstract_: None,
            cited_by: 0,
            score,
        })
    }

    pub fn into_result(self) -> ResearchPaperResult {
        let mut ids: HashMap<String, Vec<String>> = HashMap::new();
        if let Some(a) = &self.arxiv {
            ids.insert("arxiv".into(), vec![a.clone()]);
        }
        if let Some(d) = &self.doi {
            ids.insert("doi".into(), vec![d.clone()]);
        }
        if let Some(w) = &self.work_id {
            ids.insert("openalex".into(), vec![w.clone()]);
        }
        let primary_id = if let Some(a) = &self.arxiv {
            format!("arxiv:{a}")
        } else if let Some(d) = &self.doi {
            format!("doi:{d}")
        } else if let Some(w) = &self.work_id {
            w.clone()
        } else {
            self.title.clone()
        };
        let paper_id = self.work_id.clone().unwrap_or_else(|| primary_id.clone());
        ResearchPaperResult {
            paper_id,
            primary_id,
            ids,
            title: self.title,
            abstract_: self.abstract_,
            score: self.score,
            signals: None, // we can't compute Firecrawl's structural graph signals live
        }
    }
}

/// Parse one OpenAlex `/works` result object into a [`PaperHit`].
fn openalex_work_to_hit(w: &serde_json::Value) -> Option<PaperHit> {
    let title = w.get("display_name")?.as_str()?.to_string();
    let work_id = w
        .get("id")
        .and_then(|v| v.as_str())
        .and_then(|s| s.rsplit('/').next())
        .map(|s| s.to_string());
    let ids = w.get("ids");
    let doi = ids
        .and_then(|i| i.get("doi"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim_start_matches("https://doi.org/").to_string());
    // arXiv id is encoded in the DOI as 10.48550/arxiv.<id> for arXiv works
    let arxiv = doi.as_ref().and_then(|d| {
        let dl = d.to_lowercase();
        dl.strip_prefix("10.48550/arxiv.").map(norm_arxiv)
    });
    let abstract_ = w
        .get("abstract_inverted_index")
        .and_then(reconstruct_abstract);
    let cited_by = w
        .get("cited_by_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let score = w
        .get("relevance_score")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    Some(PaperHit {
        work_id,
        arxiv,
        doi,
        title,
        abstract_,
        cited_by,
        score,
    })
}

fn openalex_base(keys: &ResearchKeys<'_>) -> String {
    let mut q = String::new();
    if let Some(k) = keys.openalex_key {
        q.push_str(&format!("&api_key={k}"));
    }
    if let Some(m) = keys.openalex_mailto {
        q.push_str(&format!("&mailto={}", enc(m)));
    }
    q
}

/// OpenAlex `/works?search=` + filters → hits.
async fn openalex_search(
    keys: &ResearchKeys<'_>,
    query: &str,
    k: usize,
    f: &SearchFilters,
) -> Vec<PaperHit> {
    let mut filter = String::new();
    if let Some(from) = &f.from {
        filter.push_str(&format!(",from_publication_date:{from}"));
    }
    if let Some(to) = &f.to {
        filter.push_str(&format!(",to_publication_date:{to}"));
    }
    if let Some(a) = &f.authors {
        filter.push_str(&format!(",raw_author_name.search:{}", enc(a)));
    }
    // ponytail: `f.categories` (arXiv cat like "cs.LG") needs an arXiv-cat ->
    // OpenAlex-concept/topic map to filter on; deferred. Currently ignored.
    let filter_param = if filter.is_empty() {
        String::new()
    } else {
        format!("&filter={}", filter.trim_start_matches(','))
    };
    let url = format!(
        "https://api.openalex.org/works?search={}{}&per_page={}&select=id,display_name,ids,abstract_inverted_index,cited_by_count,relevance_score{}",
        enc(query),
        filter_param,
        k.min(50),
        openalex_base(keys),
    );
    match get_json(&url, None).await {
        Some(v) => v
            .get("results")
            .and_then(|r| r.as_array())
            .map(|arr| arr.iter().filter_map(openalex_work_to_hit).collect())
            .unwrap_or_default(),
        None => Vec::new(),
    }
}

/// Semantic Scholar `/paper/search` → hits (booster). Failures return empty.
async fn ss_search(keys: &ResearchKeys<'_>, query: &str, k: usize) -> Vec<PaperHit> {
    let url = format!(
        "https://api.semanticscholar.org/graph/v1/paper/search?query={}&limit={}&fields=title,abstract,externalIds,citationCount",
        enc(query),
        k.min(50),
    );
    let Some(v) = get_json(&url, keys.s2_key).await else {
        return Vec::new();
    };
    v.get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|p| {
                    let title = p.get("title")?.as_str()?.to_string();
                    let ext = p.get("externalIds");
                    let arxiv = ext
                        .and_then(|e| e.get("ArXiv"))
                        .and_then(|v| v.as_str())
                        .map(norm_arxiv);
                    let doi = ext
                        .and_then(|e| e.get("DOI"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    Some(PaperHit {
                        work_id: None,
                        arxiv,
                        doi,
                        title,
                        abstract_: p.get("abstract").and_then(|v| v.as_str()).map(String::from),
                        cited_by: p.get("citationCount").and_then(|v| v.as_u64()).unwrap_or(0),
                        score: 0.0,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// SS full-text snippet search → arXiv ids only (recovers body-relevant papers
/// keyword/abstract search misses). The 59.6% harness's big lever.
async fn ss_snippet_ids(keys: &ResearchKeys<'_>, query: &str) -> Vec<String> {
    let url = format!(
        "https://api.semanticscholar.org/graph/v1/snippet/search?limit=100&query={}",
        enc(query),
    );
    let Some(v) = get_json(&url, keys.s2_key).await else {
        return Vec::new();
    };
    let blob = v.to_string();
    let mut seen = std::collections::HashSet::new();
    arxiv_re()
        .find_iter(&blob)
        .map(|m| norm_arxiv(m.as_str()))
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

/// Merge candidate pools, dedup by [`PaperHit::key`], rank, cap at `k`.
/// Ranking: search-frequency (how many sources surfaced it) first, then
/// relevance score, then citation count — coverage-first, matching the harness.
pub fn merge_rank(pools: Vec<Vec<PaperHit>>, k: usize) -> Vec<ResearchPaperResult> {
    let mut by_key: HashMap<String, (PaperHit, u32)> = HashMap::new();
    for pool in pools {
        // dedup WITHIN a pool first, so an intra-pool duplicate doesn't fake
        // multi-source agreement (frequency = how many SOURCES surfaced it).
        let mut seen_in_pool = std::collections::HashSet::new();
        let unique: Vec<PaperHit> = pool
            .into_iter()
            .filter(|h| seen_in_pool.insert(h.key()))
            .collect();
        for hit in unique {
            let key = hit.key();
            by_key
                .entry(key)
                .and_modify(|(existing, freq)| {
                    *freq += 1;
                    // keep the richest record (prefer one with abstract / work_id)
                    if existing.abstract_.is_none() && hit.abstract_.is_some() {
                        existing.abstract_ = hit.abstract_.clone();
                    }
                    if existing.work_id.is_none() && hit.work_id.is_some() {
                        existing.work_id = hit.work_id.clone();
                    }
                    if existing.doi.is_none() && hit.doi.is_some() {
                        existing.doi = hit.doi.clone();
                    }
                    existing.cited_by = existing.cited_by.max(hit.cited_by);
                    existing.score = existing.score.max(hit.score);
                })
                .or_insert((hit, 1));
        }
    }
    let mut ranked: Vec<(PaperHit, u32)> = by_key.into_values().collect();
    ranked.sort_by(|(a, fa), (b, fb)| {
        fb.cmp(fa)
            .then(
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(b.cited_by.cmp(&a.cited_by))
    });
    ranked
        .into_iter()
        .take(k)
        .map(|(h, _)| h.into_result())
        .collect()
}

/// `search_papers` OpenAlex + SS legs (the route adds the SearXNG leg + calls
/// [`merge_rank`]). Returns raw pools so the route can union its own search.
pub async fn search_papers_pools(
    keys: &ResearchKeys<'_>,
    query: &str,
    k: usize,
    f: &SearchFilters,
) -> Vec<Vec<PaperHit>> {
    let (oa, ss, snip) = tokio::join!(
        openalex_search(keys, query, k, f),
        ss_search(keys, query, k),
        ss_snippet_ids(keys, query),
    );
    // snippet ids -> thin hits (arxiv only) so the union picks up body matches
    let snip_hits: Vec<PaperHit> = snip
        .into_iter()
        .map(|a| PaperHit {
            work_id: None,
            arxiv: Some(a),
            doi: None,
            title: String::new(),
            abstract_: None,
            cited_by: 0,
            score: 0.0,
        })
        .collect();
    vec![oa, ss, snip_hits]
}

/// Is `id` an arXiv-form id (`arxiv:X`, `arXiv:X`, or a bare `NNNN.NNNNN`)?
/// Returns the bare normalized arXiv id if so.
fn as_arxiv_id(id: &str) -> Option<String> {
    if id.starts_with('W') || id.starts_with("doi:") {
        return None;
    }
    let stripped = id
        .strip_prefix("arxiv:")
        .or_else(|| id.strip_prefix("arXiv:"))
        .unwrap_or(id);
    if arxiv_re().is_match(stripped) {
        Some(norm_arxiv(stripped))
    } else {
        None
    }
}

/// SS `/paper/arXiv:<id>` → metadata. SS is keyed directly by arXiv id, so it
/// resolves reliably where OpenAlex's `10.48550/arxiv.<id>` DOI lookup misses
/// (published papers carry their venue DOI in OpenAlex, not the arXiv one).
async fn ss_inspect(keys: &ResearchKeys<'_>, arxiv: &str) -> Option<ResearchPaperMeta> {
    let url = format!(
        "https://api.semanticscholar.org/graph/v1/paper/arXiv:{arxiv}?fields=title,abstract,authors,externalIds,publicationDate,fieldsOfStudy"
    );
    let v = get_json(&url, keys.s2_key).await?;
    let title = v.get("title")?.as_str()?.to_string();
    let mut ids: HashMap<String, Vec<String>> = HashMap::new();
    ids.insert("arxiv".into(), vec![arxiv.to_string()]);
    if let Some(d) = v
        .get("externalIds")
        .and_then(|e| e.get("DOI"))
        .and_then(|x| x.as_str())
    {
        ids.insert("doi".into(), vec![d.to_string()]);
    }
    let authors = v.get("authors").and_then(|a| a.as_array()).map(|arr| {
        arr.iter()
            .filter_map(|x| x.get("name")?.as_str().map(String::from))
            .collect::<Vec<_>>()
    });
    let categories = v
        .get("fieldsOfStudy")
        .and_then(|f| f.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect::<Vec<_>>()
        });
    let date = v
        .get("publicationDate")
        .and_then(|x| x.as_str())
        .map(String::from);
    Some(ResearchPaperMeta {
        paper_id: format!("arxiv:{arxiv}"),
        ids: Some(ids),
        title,
        abstract_: v.get("abstract").and_then(|x| x.as_str()).map(String::from),
        authors,
        categories,
        created_date: date.clone(),
        update_date: date,
    })
}

/// OpenAlex inspect for work ids / DOIs / arXiv-preprint-only papers.
async fn openalex_inspect(keys: &ResearchKeys<'_>, id: &str) -> Option<ResearchPaperMeta> {
    let filter = if let Some(d) = id.strip_prefix("doi:") {
        format!("filter=doi:{d}")
    } else if id.starts_with('W') {
        format!("filter=openalex_id:{id}")
    } else {
        format!("filter=doi:10.48550/arxiv.{}", norm_arxiv(id))
    };
    let url = format!(
        "https://api.openalex.org/works?{}&select=id,display_name,ids,abstract_inverted_index,authorships,primary_topic,publication_date{}",
        filter,
        openalex_base(keys),
    );
    let v = get_json(&url, None).await?;
    let w = v.get("results")?.as_array()?.first()?;
    let hit = openalex_work_to_hit(w)?;
    let authors = w.get("authorships").and_then(|a| a.as_array()).map(|arr| {
        arr.iter()
            .filter_map(|x| {
                x.get("author")?
                    .get("display_name")?
                    .as_str()
                    .map(String::from)
            })
            .collect::<Vec<_>>()
    });
    let categories = w
        .get("primary_topic")
        .and_then(|t| t.get("display_name"))
        .and_then(|v| v.as_str())
        .map(|s| vec![s.to_string()]);
    let date = w
        .get("publication_date")
        .and_then(|v| v.as_str())
        .map(String::from);
    let result = hit.into_result();
    Some(ResearchPaperMeta {
        paper_id: result.paper_id,
        ids: Some(result.ids),
        title: result.title,
        abstract_: result.abstract_,
        authors,
        categories,
        created_date: date.clone(),
        update_date: date,
    })
}

/// `GET /papers/{id}` metadata. arXiv ids resolve via Semantic Scholar (keyed by
/// arXiv); work ids / DOIs via OpenAlex. SS failure falls back to OpenAlex.
pub async fn inspect(keys: &ResearchKeys<'_>, id: &str) -> Option<ResearchPaperMeta> {
    if let Some(arxiv) = as_arxiv_id(id)
        && let Some(m) = ss_inspect(keys, &arxiv).await
    {
        return Some(m);
    }
    openalex_inspect(keys, id).await
}

/// One SS paper object (`{externalIds, title}`) → a thin [`PaperHit`] (arXiv only).
fn ss_paper_to_hit(p: &serde_json::Value) -> Option<PaperHit> {
    let arxiv = p
        .get("externalIds")?
        .get("ArXiv")?
        .as_str()
        .map(norm_arxiv)?;
    Some(PaperHit {
        work_id: None,
        arxiv: Some(arxiv),
        doi: None,
        title: p
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string(),
        abstract_: None,
        cited_by: 0,
        score: 0.0,
    })
}

/// SS citation-graph expansion → [`PaperHit`]s (with titles, so `/similar`
/// results aren't empty-titled). `ponytail:` no OpenAlex fallback yet — on SS
/// 429/failure this returns empty; the OpenAlex referenced_works/cites fallback
/// (research_tools.py `openalex_expand`) is the recall upgrade.
async fn ss_expand(keys: &ResearchKeys<'_>, arxiv: &str, mode: Mode) -> Vec<PaperHit> {
    let (path, field) = match mode {
        Mode::References => ("references", "citedPaper"),
        Mode::Citers => ("citations", "citingPaper"),
        Mode::Similar => {
            let url = format!(
                "https://api.semanticscholar.org/recommendations/v1/papers/forpaper/arXiv:{arxiv}?fields=externalIds,title&limit=100"
            );
            let Some(v) = get_json(&url, keys.s2_key).await else {
                return Vec::new();
            };
            return v
                .get("recommendedPapers")
                .and_then(|d| d.as_array())
                .map(|arr| arr.iter().filter_map(ss_paper_to_hit).collect())
                .unwrap_or_default();
        }
    };
    let url = format!(
        "https://api.semanticscholar.org/graph/v1/paper/arXiv:{arxiv}/{path}?fields={field}.externalIds,{field}.title&limit=100"
    );
    let Some(v) = get_json(&url, keys.s2_key).await else {
        return Vec::new();
    };
    v.get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|row| ss_paper_to_hit(row.get(field)?))
                .collect()
        })
        .unwrap_or_default()
}

/// `GET /papers/{id}/similar` — citation-graph expansion → ranked results.
/// `mode` selects references / citers / similar. Accepts an `arxiv:`-prefixed,
/// bare, or versioned id (normalized).
pub async fn related(
    keys: &ResearchKeys<'_>,
    id: &str,
    mode: Mode,
    k: usize,
) -> Vec<ResearchPaperResult> {
    let aid = norm_arxiv(id);
    let hits: Vec<PaperHit> = ss_expand(keys, &aid, mode)
        .await
        .into_iter()
        .filter(|h| h.arxiv.as_deref() != Some(aid.as_str()))
        .collect();
    merge_rank(vec![hits], k)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn arxiv_extraction_and_norm() {
        assert_eq!(norm_arxiv("2105.05233v3"), "2105.05233");
        // the critical fix: prefixed ids must NOT be split on 'v'
        assert_eq!(norm_arxiv("arXiv:1706.03762"), "1706.03762");
        assert_eq!(norm_arxiv("arxiv:2105.05233v12"), "2105.05233");
        let ids: Vec<_> = arxiv_re()
            .find_iter("see 1706.03762 and arXiv:2301.07041v2")
            .map(|m| norm_arxiv(m.as_str()))
            .collect();
        assert_eq!(ids, vec!["1706.03762", "2301.07041"]);
    }

    #[test]
    fn abstract_reconstruction() {
        let inv = json!({"Fully": [0], "Homomorphic": [1], "Encryption": [2], "is": [3]});
        assert_eq!(
            reconstruct_abstract(&inv).unwrap(),
            "Fully Homomorphic Encryption is"
        );
    }

    #[test]
    fn openalex_work_maps_arxiv_from_doi() {
        let w = json!({
            "id": "https://openalex.org/W123",
            "display_name": "Attention Is All You Need",
            "ids": {"doi": "https://doi.org/10.48550/arXiv.1706.03762"},
            "cited_by_count": 99999,
            "relevance_score": 12.3
        });
        let h = openalex_work_to_hit(&w).unwrap();
        assert_eq!(h.work_id.as_deref(), Some("W123"));
        assert_eq!(h.arxiv.as_deref(), Some("1706.03762"));
        let r = h.into_result();
        assert_eq!(r.primary_id, "arxiv:1706.03762");
        assert_eq!(r.paper_id, "W123");
        assert_eq!(r.ids["arxiv"][0], "1706.03762");
    }

    /// Live end-to-end smoke test against real OpenAlex + Semantic Scholar.
    /// Ignored by default (network). Run with keys in env:
    ///   OPENALEX_KEY=.. S2_KEY=.. cargo test -p crw-search live_smoke -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_smoke() {
        let oa = std::env::var("OPENALEX_KEY").ok();
        let s2 = std::env::var("S2_KEY").ok();
        let keys = ResearchKeys {
            openalex_key: oa.as_deref(),
            openalex_mailto: Some("team@fastcrw.com"),
            s2_key: s2.as_deref(),
        };
        // inspect a famous paper
        let meta = inspect(&keys, "arxiv:1706.03762").await.expect("inspect");
        println!("inspect title: {}", meta.title);
        assert!(meta.title.to_lowercase().contains("attention"));
        assert!(meta.authors.as_ref().is_some_and(|a| !a.is_empty()));

        // search merges OpenAlex + SS
        let pools = search_papers_pools(
            &keys,
            "flash attention efficient transformers",
            20,
            &SearchFilters::default(),
        )
        .await;
        let results = merge_rank(pools, 20);
        println!("search returned {} papers", results.len());
        assert!(!results.is_empty(), "search returned nothing");

        // citation graph (references of the transformer paper)
        let refs = related(&keys, "1706.03762", Mode::References, 20).await;
        println!("references: {}", refs.len());
    }

    #[test]
    fn merge_rank_dedups_and_orders_by_frequency() {
        let a = PaperHit {
            work_id: None,
            arxiv: Some("1.1".into()),
            doi: None,
            title: "A".into(),
            abstract_: None,
            cited_by: 1,
            score: 0.0,
        };
        let a2 = PaperHit {
            work_id: None,
            arxiv: Some("1.1".into()),
            doi: None,
            title: "A".into(),
            abstract_: Some("x".into()),
            cited_by: 5,
            score: 0.0,
        };
        let b = PaperHit {
            work_id: None,
            arxiv: Some("2.2".into()),
            doi: None,
            title: "B".into(),
            abstract_: None,
            cited_by: 99,
            score: 0.0,
        };
        let out = merge_rank(vec![vec![a, b], vec![a2]], 10);
        assert_eq!(out.len(), 2);
        // "1.1" appears in 2 pools -> ranks first despite lower citations
        assert_eq!(out[0].primary_id, "arxiv:1.1");
        assert_eq!(out[0].abstract_.as_deref(), Some("x")); // merged the richer record
    }
}
