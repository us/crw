# Research API (Firecrawl-compatible) — Implementation Plan

## Context

We beat Firecrawl's Research Index on the ArXivQA benchmark (fastCRW **59.6%** vs Firecrawl **53.3%**, 191 queries, cheat-free) using a benchmark agent over `arxivqa-bench/research_tools.py` — a **live-API cascade** (fastCRW research-mode search + OpenAlex + Semantic Scholar + arXiv scrape + citation expansion). No self-hosted index.

Goal: ship that capability as **Firecrawl-Research-API-compatible HTTP endpoints** so Firecrawl SDK/CLI calls work against our base URL. Firecrawl's endpoints are **stateless primitives**; the agent intelligence (intent routing, exact-name reframing, leaderboard/survey methods) lives in the **SKILL**. We mirror that split.

> **Legal posture (user decision, 2026-06-19):** review flagged SS API license (no commercial re-distribution) + arXiv ToU (no full-text serving). **User accepts the risk and will handle licensing later — NOT an engineering blocker.** So we keep the FULL 59.6% stack: SS snippet + SS citations + arXiv full-text. `// ponytail: legal deferred to user, not our gate.`
>
> **Primary recall driver = our OWN fastCRW search** (web google/bing via SearXNG + research-mode scholarly engines), in-process via `state.searxng`. OpenAlex (CC0) + SS are boosters. This is the honest source order from `research_tools.py`: every reframing hits fastCRW search first; SS snippet is one pass; SS/OpenAlex citation-expansion + arXiv PDF selfrefs layer on top.

**Verified facts (from research + review agents):**
- Engine routes registered in `crw-server/src/routes/v1/mod.rs` `router()`; axum 0.8 path-param syntax `{id}`. Handlers `pub async fn(State<AppState>, Path, Query) -> Result<Json<..>, AppError>`. `AppState` (`state.rs:118`) = `config: Arc<AppConfig>`, `renderer: Arc<FallbackRenderer>`, `searxng: Option<Arc<SearxngClient>>`. **No shared reqwest client** — each integration owns one (`crw-search/src/wikidata.rs` = `OnceLock<reqwest::Client>` + `moka::future::Cache` + `Semaphore`; **note: wikidata.rs has NO retry/backoff — that's net-new code**).
- Config: `SearchConfig` (`config.rs:197`), env `CRW_SEARCH__*`. Types in `crw-core/src/types.rs`, `#[serde(rename_all="camelCase")]`, no utoipa. `abstract` is a Rust keyword → `#[serde(rename="abstract")] abstract_`.
- **`crw-search` does NOT depend on `crw-crawl`** (verified Cargo.toml). So `scrape_url` (`crw-crawl/src/single.rs`, called at `search.rs:1108`) is **not callable from `crw-search/research.rs`** without a new dep (circular-risk). Any scrape-needing logic lives in the **route handler** (`crw-server`, which already has `renderer` + depends on crw-crawl), not the research client.
- SaaS proxies via `createProxyHandler` (`api-handler.ts:58`) → `crwFetch` (`crw-client.ts:10`) to internal `crw-api-internal:3000`. **`crwFetch` has NO query-string support today** (POST+body only) — net-new. The pipeline acquires a concurrency slot **only for POST** (`api-pipeline.ts:288`); **GET endpoints bypass the limiter entirely** — so research GET fan-out would be unthrottled. `checkAndConsumeQuota` (`usage.ts:1299`) is a single reserve+commit; there is **no reserve-k/refund-unused split** for GET (the LLM path's `commitLlmReserve` is search-specific). `createProxyHandler` does **not** thread `requestId` into billing → research handlers must use `withApiPipeline` directly, not `createProxyHandler`.
- Engine keys → `crw-api` container via `docker-compose.prod.yml` env (`CRW_SEARCH__OPENALEX_API_KEY: ${OPENALEX_API_KEY:-}`). SaaS doesn't need them.

**Firecrawl exact I/O to match (drop-in) — verified against the Firecrawl v2 SDK source:**
- `GET /v2/search/research/papers?query&k&authors&categories&from&to` → `{success, results:[PaperResult]}`
- `GET /v2/search/research/papers/{id}` → `{success, paper:PaperMetadata}` ; with `?query&k` → `{success, paper:PaperMetadata, paperId, query, passages:[{text, score}]}`
- `GET /v2/search/research/papers/{id}/similar?intent&mode=similar|citers|references&k&rerank&anchor` → `{success, results:[PaperResult], poolSize, truncated, note}`
- `GET /v2/search/research/github?query&k` → `{success, results:[GitHubItem]}`
- **Two distinct paper types** (do NOT merge): `PaperResult` = `{paperId, primaryId, ids, title, abstract, score, signals?}`; `PaperMetadata` = `{paperId, ids?, title, abstract, authors?, categories?, createdDate?, updateDate?}` (NO score/signals; HAS authors/categories/dates).
- `signals?` is OPTIONAL; if present, all four fields (`structural, semantic, articleRank, seedOverlap`) are `number` (NOT nullable). → **omit the whole `signals` object** rather than send nulls.
- `ids` = `Record<string,string[]>` with **prefix-less** values: `{"arxiv":["2105.05233"]}`; `primaryId` IS prefixed: `"arxiv:2105.05233"`.
- `similar`: `intent` is REQUIRED; `rerank?:boolean`, `anchor?:string[]` exist.
- `GitHubItem` = `{resultType:"github_history"|"repo_readme"|"web", repo, url, pageType, number, segmentCount, readmeUrl?, title, snippet, contentMd, scores?:{semantic?,lexical?,fusion?,rrf?,rerank?}}`.
- Auth: BOTH Firecrawl and us require `Authorization: Bearer <key>` (Firecrawl `fc-*`, us `crw_live_*`). Header format matches → SDK `apiKey` + base-url override is enough; no keyless path needed.

**What we are NOT doing (scope guard):** no self-hosted index, no embeddings, no agent loop in endpoints, no LLM calls in endpoints, no canonical numeric `paperId`. (SS + arXiv full-text ARE in scope — user accepted the ToS risk.)

## Approach
1. **Phase 0 first (de-risk):** lock the `paperId` scheme + a Firecrawl-SDK contract harness + the SS/arXiv legal decision BEFORE porting. (Review consensus: doing this after Phase 1 forces rewrites.)
2. **Engine logic** (Rust, `crw-opencore`): `search_papers` = our own fastCRW search (in-process `state.searxng`: web + research-mode) merged with OpenAlex + SS; `crw-search/src/research.rs` (OnceLock reqwest + moka + Semaphore + net-new backoff) owns the OpenAlex + SS HTTP calls; the **route handler** owns arXiv scrape (renderer lives there, not in crw-search).
3. **SaaS proxies + bills** (`crw-saas`): GET routes at `/v2/search/research/*` via `withApiPipeline` (not `createProxyHandler`), with net-new GET query-string `crwFetch`, an explicit research concurrency cap, and status-defined debit/refund rules.
4. **Response shape = Firecrawl's exactly** (two paper types, `signals` omitted, prefix-less `ids`).
5. **SKILL + MCP**: rewrite `fastcrw-research-index/SKILL.md` to drive the hosted endpoints; MCP tools `crw_research_*`.

## Phases

### Phase 0 — De-risk: id scheme, contract harness, legal sign-off (NEW, do first)
- **`paperId` scheme**: must be URL-safe and round-trip through `GET /papers/{id}` for ALL ids including legacy arXiv (`hep-th/9901001`, contains `/`) and DOIs (`10.48550/...`, contains `/`). Decision: **`paperId` = the OpenAlex work id** (`W2105...`, opaque, URL-safe, stable); `primaryId` = `"arxiv:<id>"` or `"doi:<doi>"`; `ids` carries the prefix-less source ids. `GET /papers/{id}` accepts EITHER a `W…` id OR a `arxiv:`/`doi:` primaryId (resolve to the work). Contract-test all three id forms + URL-encoding.
- **Contract harness**: a script that points the real Firecrawl Node + Python SDK at our base URL and asserts each method (`searchPapers/getPaper/similarPapers/searchGithub`) deserializes without error, plus golden JSON fixtures captured from Firecrawl's live API for shape-diff. This is the drop-in gate.
- **Legal sign-off**: confirm OpenAlex-primary (drop SS) + abstract-only read. Document attribution ("powered by OpenAlex") in docs.
- Effort: 0.5d. Risk: low (but unblocks everything).

### Phase 1 — Engine: research client + config (`crw-opencore`)
- **`crw-core/src/config.rs`** (`SearchConfig` :197): add `openalex_api_key`, `openalex_mailto`, `s2_api_key` (all `Option<String>`, `#[serde(default)]`). Env `CRW_SEARCH__OPENALEX_API_KEY`, `CRW_SEARCH__OPENALEX_MAILTO`, `CRW_SEARCH__S2_API_KEY`.
- **`crw-search/src/research.rs` (new)**: mirror `wikidata.rs` (`OnceLock<reqwest::Client>` + `moka` + `Semaphore`) PLUS net-new exponential backoff (no crate; `tokio::time::sleep` loop on 429/5xx, per `research_tools.py` `_post`/`ss_snippet`). **Split moka caches with byte-aware `weigher`**: metadata vs citation-list (sizes vary 100x). Functions:
  - `search_papers(query,k,filters) -> Vec<PaperHit>` — **merge of: (a) our own fastCRW search via `state.searxng` — web (google/bing) + research-mode scholarly engines, the primary driver; (b) OpenAlex `/works?search=` + filters; (c) SS `/paper/search` + `/snippet/search` (full-text booster)**. Dedup by arxiv/doi, rank by source-frequency + `cited_by_count`. OpenAlex filters: `from`/`to`→`*_publication_date`, `authors`→`authorships.author.id`, `categories`→arXiv-cat→OpenAlex-concept map (**net-new ~30 lines**). Reconstruct OpenAlex `abstract` from `abstract_inverted_index` (**net-new ~20 lines**). (Note: the SearXNG search call is in-process from the route, passed into this merge — see Phase 2; research.rs owns only the OpenAlex+SS legs.)
  - `inspect(work_or_primary_id) -> PaperMeta` — OpenAlex work lookup (or SS `/paper/arXiv:<id>`) → metadata.
  - `related(id,mode,intent,k) -> Vec<PaperHit>` — `references`: SS `/paper/arXiv:<id>/references` + OpenAlex `referenced_works` + arXiv PDF selfrefs (route-side scrape); `citers`: SS `/citations` + OpenAlex `cites:<wid>`; `similar`: SS `/recommendations/.../forpaper/arXiv:<id>` + OpenAlex `related_works`.
- **arXiv scrape (selfrefs, read full-text) stays OUT of this crate** (dep boundary) — route handler does it via `state.renderer`, passes ids in. See Phase 2.
- Rate-limit: OpenAlex polite pool (`mailto`+`api_key`) ~10 req/s; SS 1 RPS shared key → Semaphore + backoff (SS is a booster, failures degrade gracefully to fastCRW-search+OpenAlex); 24h cache; per-source timeout 5s, partial on timeout.
- Effort: **3–4d** (abstract reconstruction, category map, backoff, 3-source merge/rank, id-normalization, tests all net-new). Risk: high (the integration phase).

### Phase 2 — Engine: routes + types (`crw-opencore`)
- **`crw-core/src/types.rs`**: add `ResearchPaperResult{paperId, primaryId, ids:HashMap<String,Vec<String>>, title, abstract_, score, signals:Option<ResearchSignals>}`, **separate** `ResearchPaperMeta{paperId, ids, title, abstract_, authors, categories, createdDate, updateDate}`, `Passage{text, score}`, `ResearchGithubItem{...}`, response wrappers. `signals` defaults to `None` (omitted). `#[serde(rename="abstract")]`.
- **`crw-server/src/routes/research.rs` (new)**: 4 GET handlers; these own the SearXNG search call (`state.searxng`) + arXiv scrape (`state.renderer`), then call into `research.rs` for OpenAlex/SS legs and merge. `search_papers` runs the in-process fastCRW search + research.rs OpenAlex/SS merge. `get_paper` branches on `query`: absent → `inspect`; present → **read_passages**: SS `/snippet/search` for the paper + arXiv `/html`|`/pdf` scrape, chunk + rank vs query → top-k `{text, score}` (cache bodies in moka). `similar` validates `intent` present (400 if missing), accepts `rerank`/`anchor`; runs SS+OpenAlex+arXiv-selfrefs merge.
- **`routes/mod.rs`** + **`v1/mod.rs`** `router()`: register the 4 GET routes; `pub mod research;`.
- Effort: 1.5d. Risk: med (the dual-mode handler + id resolution).

### Phase 3 — SaaS: proxy + billing (`crw-saas`)
- **`crw-client.ts`**: extend `crwFetch` with `{method, query}` → builds `new URL` + `searchParams` (net-new). Don't send `Content-Type: application/json` on GET.
- **4 GET routes** `src/app/api/v2/search/research/{papers,papers/[id],papers/[id]/similar,github}/route.ts`: use `withApiPipeline` directly (NOT `createProxyHandler` — it can't thread requestId/GET billing). Read `searchParams`. Acquire a **dedicated research concurrency slot** (small semaphore, e.g. 4) — do NOT reuse the POST `SEARCH_MAX_CONCURRENT` (GETs bypass it) and do NOT leave research unthrottled.
- **Billing rules (status-defined)**: reserve `min(k, CAP)` credits (CAP e.g. 50) before upstream; after response, **refund `reserved - actual_results`**; full refund on upstream 5xx/timeout; cache-hit still charges (it's a served result); empty result → refund all but a 1-credit floor. Add a `research` credit line to the pricing map (own entry, not `llm-pricing.ts`).
- Effort: **2d** (was 1d — GET pipeline + reserve/refund are net-new). Risk: med.

### Phase 4 — SDKs (`crw-opencore/sdks`)
- **TS** `client.ts`+`types.ts` and **Python** `client.py`: `research.searchPapers/getPaper/similarPapers/searchGithub` (TS) + snake_case (Python), `httpRequest("GET", "/v2/search/research/...")`, mirroring Firecrawl SDK method names. Effort: 0.5d. Risk: low.

### Phase 5 — SKILL + MCP (`crw-opencore` / arxivqa-bench)
- **`fastcrw-research-index/SKILL.md`**: rewrite to drive the hosted endpoints (`crw_research_*`). Keep intent-routing + exact-name reframing + leaderboard/survey (the brain). **Note honestly**: the hosted product is OpenAlex-backed; the local benchmark used SS too — the skill must not promise SS-level full-text recall.
- **`crw-mcp`**: add `crw_research_search_papers`, `crw_research_paper`, `crw_research_similar`, `crw_research_github`.
- Effort: 0.5d. Risk: low.

### Phase 6 — Deploy + docs/marketing
- Engine keys → host `.env` + compose crw-api env. Push opencore main → engine-redeploy; push saas → deploy; warm `/api/health`.
- Docs: research API page (per-endpoint cURL/SDK), `llms.txt`, **OpenAlex attribution**. Avoid implying Firecrawl affiliation; market as "compatible with the Firecrawl research search API", scoped to what's actually drop-in.
- Blog + chart (`arxivqa-bench/results/final.jsonl`). **Honest framing**: the 59.6% is the *agent+SKILL over the live stack* (incl. SS, local) — label the chart "fastCRW research skill + endpoints", and re-measure the OpenAlex-only hosted product separately before quoting a product number.
- Effort: 1d. Risk: low.

## Verification
- **Phase 0 gate**: Firecrawl Node + Python SDK pointed at our base URL → all 4 methods deserialize; golden-fixture shape-diff passes; `paperId` round-trips for `W…`, `arxiv:…`, `doi:…`, legacy `hep-th/…` (URL-encoded), and DOI-with-slash.
- **Per endpoint**: `curl` ours vs Firecrawl's same call → key/type diff. Failure cases: malformed id, DOI id, old arXiv id, no-abstract paper, upstream 429/500, slow/empty results, empty GitHub.
- **Recall**: re-run `arxivqa-bench` with the SKILL on the hosted endpoints → record the **OpenAlex-only product number** (expected below 59.6%; quantify the gap honestly). `score.py` vs `final.jsonl`.
- **Engine**: `cargo build`, `cargo clippy -D warnings`, `cargo test -p crw-search research` — unit tests: abstract-inverted-index reconstruction, arxiv↔doi↔workid normalization, category-map, serde round-trip (both paper types, `signals` omitted).
- **Billing**: reserve→partial-refund math per status (5xx full refund, partial results partial refund, cache hit charges, empty floors at 1).
- **Load**: sustained concurrency at the research cap with simulated OpenAlex 429 → no 502, bounded latency, partial-result fallback fires.

## Open questions
0. **LEGAL — RESOLVED (user, 2026-06-19)**: keep full stack (SS + arXiv full-text); user accepts ToS risk and will handle licensing later. Not an engineering gate.
1. **paperId = OpenAlex work id** — confirms URL-safe round-trip; does any Firecrawl SDK code path assume numeric? (Phase 0 harness answers this.)
2. **read_passages**: SS snippet + arXiv scrape per call is multi-second on cold cache. Acceptable for v1 with moka body-cache, or add a fast abstract-only fallback when scrape is slow?
3. **GitHub endpoint**: our `categories:["github"]` SearXNG search → does it yield issues/PRs/discussions or just repos? If repo-only, `resultType` is always `repo_readme` and we under-deliver vs Firecrawl. Verify coverage; document the gap.
4. **Product recall**: re-run `arxivqa-bench` with the SKILL on the hosted endpoints (full stack) → confirm ≈59.6% (not worse than the local-tool run). If the in-process merge differs from research_tools.py, reconcile.
5. **Rate-limit at scale**: SS 1 RPS shared key + OpenAlex daily budget under concurrency. Caching + graceful degradation (fastCRW-search+OpenAlex carry when SS 429s) covers v1; self-host index is the scale answer (hundreds of users/day).

## Order of execution & risk table
| Phase | Files | Effort | Risk | Order |
|---|---|---|---|---|
| 0 De-risk (id, harness, legal) | contract-harness script, fixtures, this doc | 0.5d | low | 1 |
| 1 Engine client+config | `crw-core/config.rs`, `crw-search/research.rs` | 3–4d | high | 2 |
| 2 Engine routes+types | `crw-core/types.rs`, `crw-server/routes/research.rs`,`v1/mod.rs` | 1.5d | med | 3 |
| 3 SaaS proxy+billing | `app/api/v2/search/research/*`, `crw-client.ts`, pricing | 2d | med | 4 |
| 4 SDKs | `sdks/{ts,py}` | 0.5d | low | 5 |
| 5 SKILL+MCP | `fastcrw-research-index/SKILL.md`, `crw-mcp` | 0.5d | low | 6 |
| 6 Deploy+docs | `.env`, compose, docs, blog | 1d | low | 7 |

## Iteration log
- **Iteration 1** (2026-06-19): 5 reviewers + Codex. Addressed 9 critical / 13 warning: added Phase 0 (paperId=OpenAlex-work-id for URL-safe round-trip incl. legacy/DOI ids, Firecrawl-SDK contract harness); split `PaperResult` vs `PaperMetadata` (two types); `signals` omitted not null; prefix-less `ids`; `intent` required + `rerank`/`anchor` added; fixed crw-search↛crw-crawl dep boundary (scrape in route handler); net-new flagged: abstract-inverted-index reconstruction, backoff, category map; corrected effort (Phase 1 1.5d→3-4d, Phase 3 1d→2d); SaaS GET billing made explicit (query-string crwFetch, dedicated research slot since GET bypasses limiter, status-defined reserve/refund). Initially pivoted to OpenAlex-only over SS/arXiv ToS.
- **Iteration 2** (2026-06-19, user steer): **reverted the legal pivot** — user accepts ToS risk, will handle licensing later, so the FULL 59.6% stack stays (SS snippet + SS citations + arXiv full-text passages + selfrefs). **Corrected the recall model**: foregrounded our OWN fastCRW search (in-process `state.searxng`: web google/bing + research-mode) as the PRIMARY `search_papers` driver — it was under-weighted in iter 1. SS/OpenAlex are boosters; graceful degradation when SS 429s. Restored `s2_api_key` config, SS in `related`/`read_passages`. Open Q #0 resolved.
