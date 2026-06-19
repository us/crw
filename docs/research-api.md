# Research API

A Firecrawl-compatible, purpose-built API for scientific-research agents:
search papers, inspect metadata, read passages, expand the citation graph, and
search research-related GitHub. Stateless primitives over **live** data
(OpenAlex + Semantic Scholar + our own SearXNG search) — no self-hosted index.

Drop-in for the Firecrawl research SDK: point it at `https://api.fastcrw.com`
and the `/v2/search/research/*` surface matches.

## Auth

`Authorization: Bearer <crw_live_…>` (cloud). Self-host serves the same shapes
at the engine path `/v1/search/research/*`.

## Endpoints

| Task | Endpoint |
|---|---|
| Search papers | `GET /v2/search/research/papers` |
| Inspect metadata / read passages | `GET /v2/search/research/papers/{id}` |
| Find related papers | `GET /v2/search/research/papers/{id}/similar` |
| Search GitHub | `GET /v2/search/research/github` |

### Search papers

`GET /v2/search/research/papers?query=&k=&authors=&categories=&from=&to=`

```bash
curl -s -H "Authorization: Bearer $FASTCRW_API_KEY" \
  "https://api.fastcrw.com/v2/search/research/papers?query=diffusion%20image%20synthesis&k=20"
```

Returns ranked papers. `id` is `paperId` (the OpenAlex work id when known, else
`arxiv:<id>`); `primaryId` is the preferred source id (`arxiv:2105.05233`);
`ids` holds the prefix-less source ids.

```json
{ "success": true, "results": [
  { "paperId": "W2105…", "primaryId": "arxiv:2105.05233",
    "ids": { "arxiv": ["2105.05233"] },
    "title": "…", "abstract": "…", "score": 0.42 }
] }
```

Filters: `authors` (substring), `categories`, `from`/`to` (`YYYY-MM-DD`).

### Inspect a paper / read passages

`GET /v2/search/research/papers/{id}` → metadata (`authors`, `categories`,
`createdDate`, …). arXiv ids resolve via Semantic Scholar; work ids / DOIs via
OpenAlex.

Add `?query=` to return the top passages answering a question (abstract-scoped):

```bash
curl -s -H "Authorization: Bearer $FASTCRW_API_KEY" \
  "https://api.fastcrw.com/v2/search/research/papers/arxiv:1706.03762?query=what%20is%20the%20attention%20mechanism&k=4"
```

### Find related papers

`GET /v2/search/research/papers/{id}/similar?intent=&mode=similar|citers|references&k=`

`intent` is required. `mode`: `similar` (recommendations), `citers` (papers that
cite the seed), `references` (papers the seed cites).

### Search GitHub

`GET /v2/search/research/github?query=&k=` → repository/README hits.

## SDK

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

## Notes / limits

- **Live, no index.** Recall comes from merging our own SearXNG search (web +
  research-mode) with OpenAlex (CC0) + Semantic Scholar. Latency is seconds, not
  the ms of a hot index.
- **read passages** are abstract-scoped today (full arXiv-body passages are a
  follow-up).
- **GitHub** results are repo/README-scoped (issue/PR granularity is a follow-up).
- The agent brain (intent routing, exact-name reframing, leaderboard/survey
  harvesting) lives in the **research skill**, not these endpoints — the skill
  over these primitives is what reproduces the 59.6% ArXivQA recall.
