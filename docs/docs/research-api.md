# Research API

A Firecrawl-compatible, purpose-built API for scientific-research agents: search
papers, inspect metadata, read passages, expand the citation graph, and search
research-related GitHub. The endpoint surface mirrors Firecrawl's research API,
so the Firecrawl research SDK/CLI works drop-in against `https://api.fastcrw.com`.

On the [ArXivQA benchmark](https://fastcrw.com/benchmarks/arxivqa-research-recall)
this stack reaches **61.0% recall — ahead of Firecrawl's Research Index (53.3%)**.
It's **live**: results come from merging our own SearXNG search (web + research
mode) with live open scholarly sources, including full-text paper search — no
self-hosted paper index.

## Authentication

Use a Bearer token: `Authorization: Bearer <crw_live_…>`. Get a key from the
[dashboard](https://fastcrw.com/dashboard). Self-host serves the same shapes at
the engine path `/v1/search/research/*`.

## Endpoints

| Task | Endpoint |
|------|----------|
| Search papers | `GET /v2/search/research/papers` |
| Inspect metadata / read passages | `GET /v2/search/research/papers/{id}` |
| Find related papers | `GET /v2/search/research/papers/{id}/similar` |
| Search GitHub | `GET /v2/search/research/github` |

## Search papers

`GET /v2/search/research/papers?query=&k=&authors=&categories=&from=&to=`

```bash
curl -s -H "Authorization: Bearer $FASTCRW_API_KEY" \
  "https://api.fastcrw.com/v2/search/research/papers?query=diffusion%20image%20synthesis&k=20"
```

Returns ranked papers. `paperId` is the canonical id (a stable work id when
known, else `arxiv:<id>`); `primaryId` is the preferred source id
(`arxiv:2105.05233`); `ids` holds the prefix-less source ids.

```json
{ "success": true, "results": [
  { "paperId": "W2105…", "primaryId": "arxiv:2105.05233",
    "ids": { "arxiv": ["2105.05233"] },
    "title": "…", "abstract": "…", "score": 0.42 }
] }
```

Filters: `authors` (substring), `categories`, `from` / `to` (`YYYY-MM-DD`).

## Inspect a paper / read passages

`GET /v2/search/research/papers/{id}` returns metadata (`authors`, `categories`,
`createdDate`, …). Accepts an arXiv id, a work id, or a DOI. Add `?query=` to
return the top passages answering a question:

```bash
curl -s -H "Authorization: Bearer $FASTCRW_API_KEY" \
  "https://api.fastcrw.com/v2/search/research/papers/arxiv:1706.03762?query=what%20is%20the%20attention%20mechanism&k=4"
```

## Find related papers

`GET /v2/search/research/papers/{id}/similar?intent=&mode=similar|citers|references&k=`

`intent` is required. `mode` selects the expansion: `similar` (recommendations),
`citers` (papers that cite the seed), `references` (papers the seed cites).

## Search GitHub

`GET /v2/search/research/github?query=&k=` returns repository/README hits for
implementation notes and engineering prior art.

## SDK

The Firecrawl research SDK works unchanged — set the base URL to
`https://api.fastcrw.com`. Or use the fastCRW SDK:

```python
from crw import CrwClient
c = CrwClient(api_key="crw_live_…")
c.search_papers("diffusion image synthesis", k=20)
c.get_paper("arxiv:1706.03762", query="what is the attention mechanism")
c.related_papers("arxiv:1706.03762", intent="efficient transformers", mode="references")
c.search_github("flash attention implementation notes")
```

```ts
import { CrwClient } from "@fastcrw/sdk";
const c = new CrwClient({ apiKey: "crw_live_…" });
await c.research.searchPapers("diffusion image synthesis", { k: 20 });
await c.research.similarPapers("arxiv:1706.03762", { intent: "efficient transformers", mode: "references" });
```

## The research skill

The endpoints are stateless primitives. The intelligence that reaches 61.0% on
ArXivQA — intent routing, exact-name query reframing, reading a leaderboard,
pulling a paper's self-references for "compare-against" questions — lives in the
**research skill** over these endpoints, the same way Firecrawl splits its
Research Index endpoints from its research skill.

Install it into your agent (Claude Code, Cursor, Codex, Gemini CLI, …):

```bash
npx skills add us/crw@crw-research
```

## Notes and limits

- **Live, no index.** Recall comes from merging our own SearXNG search (web +
  research mode) with live open scholarly sources + full-text paper search.
  Latency is seconds, not the milliseconds of a hot index.
- **Read passages** are abstract-scoped today; full arXiv-body passages are on
  the roadmap.
- **GitHub** results are repo/README-scoped today.
- Powered by open scholarly sources.
