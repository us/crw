# Recipe: Build a RAG Knowledge Base from a Docs Site

Turn any documentation website into a queryable knowledge base in four steps:
map the site to discover all URLs, scrape every page with built-in topic chunking,
embed the chunks with a local or hosted embeddings model, then store and query them
in a vector database.

**Target site used in this recipe:** `https://docs.pydantic.dev/latest/` (~120 pages,
publicly crawlable, no JS required, good real-world docs structure).

**What you will build:**

```
docs site  →  map URLs  →  scrape each URL + chunk  →  embed  →  Chroma / pgvector  →  query
```

**Prerequisites:**

```bash
pip install crw chromadb openai          # vector DB + embeddings
# or: pip install crw psycopg2-binary    # if you prefer pgvector
export CRW_API_KEY="crw-..."
export OPENAI_API_KEY="sk-..."           # for text-embedding-3-small
```

---

## Step 1: Discover all documentation URLs

Use `/v1/map` to find every URL on the site before fetching any content. This is
faster and cheaper than crawling blind — you know the full scope up front and can
filter before spending credits on scrapes.

:::tabs
::tab{title="Python"}
```python
import os
from crw import CrwClient

client = CrwClient(api_key=os.environ["CRW_API_KEY"])

urls = client.map(
    "https://docs.pydantic.dev/latest/",
    max_depth=3,
    use_sitemap=True,           # try sitemap.xml first, BFS as fallback
)

# Keep only docs pages (drop anchors, changelogs, API refs if unwanted)
doc_urls = [u for u in urls if "/api/" not in u and "#" not in u]
print(f"Found {len(doc_urls)} documentation pages")
# Found 87 documentation pages
```
::

::tab{title="cURL"}
```bash
curl -s -X POST https://api.fastcrw.com/v1/map \
  -H "Authorization: Bearer $CRW_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://docs.pydantic.dev/latest/",
    "maxDepth": 3,
    "useSitemap": true
  }' | jq '.data.links | length'
# 87
```
::
:::

**Expected response shape:**

```json
{
  "success": true,
  "data": {
    "links": [
      "https://docs.pydantic.dev/latest/",
      "https://docs.pydantic.dev/latest/concepts/models/",
      "..."
    ]
  }
}
```

---

## Step 2: Scrape pages and chunk each one

Scrape each discovered URL with `POST /v1/scrape`, passing `chunkStrategy` to
split the page server-side (one chunk per heading section). The engine returns a
`chunks` array on each result — no text-splitting library needed.

> **Why not batch_scrape?** `POST /firecrawl/v2/batch/scrape` does not forward
> `chunkStrategy` to the engine and never returns a `chunks` field. Use
> individual `/v1/scrape` calls (looped) to get chunked output.

**`chunkStrategy` options:**

| type | splits on | best for |
|------|-----------|----------|
| `"topic"` | markdown headings (`#`, `##`, ...) | documentation, wikis |
| `"sentence"` | sentence boundaries `.!?` | articles, prose |
| `"regex"` | custom pattern | custom separators |

:::tabs
::tab{title="Python"}
```python
# POST /v1/scrape for each URL — chunkStrategy is a v1 parameter
pages = []
for url in doc_urls[:50]:          # start small; remove slice for full run
    page = client.scrape(
        url,
        formats=["markdown"],
        chunkStrategy={              # split on markdown headings
            "type": "topic",
            "maxChars": 1500,        # hard-cap each chunk at 1500 chars
            "overlapChars": 0,
            "dedupe": True,          # drop near-duplicate chunks (Jaccard > 85%)
        },
        # only_main_content=True is the default — no need to pass it explicitly
    )
    pages.append(page)

print(f"Scraped {len(pages)} pages")
# Scraped 50 pages
```
::

::tab{title="cURL"}
```bash
# POST /v1/scrape with chunkStrategy (one call per URL)
curl -s -X POST https://api.fastcrw.com/v1/scrape \
  -H "Authorization: Bearer $CRW_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://docs.pydantic.dev/latest/concepts/models/",
    "formats": ["markdown"],
    "chunkStrategy": { "type": "topic", "maxChars": 1500, "dedupe": true },
    "onlyMainContent": true
  }' | jq '.data.chunks | length'
# 3
```
::
:::

**Expected shape of one page result:**

```json
{
  "markdown": "# Models\n\nPydantic models are...",
  "chunks": [
    { "index": 0, "content": "# Models\n\nPydantic models are the primary way...", "score": null },
    { "index": 1, "content": "## Basic Model Usage\n\nThe simplest Pydantic model...", "score": null },
    { "index": 2, "content": "## Field Definitions\n\nFields are defined as class attributes...", "score": null }
  ],
  "metadata": {
    "title": "Models - Pydantic",
    "sourceURL": "https://docs.pydantic.dev/latest/concepts/models/",
    "statusCode": 200
  }
}
```

> **Note:** `score` is `null` when no `query` + `filterMode` are passed. Scores
> appear when you do server-side BM25/cosine filtering — useful for one-shot QA but
> not for building a knowledge base (you want all chunks indexed).

---

## Step 3: Embed the chunks

Flatten all chunks from all pages into a single list, then embed them in batches.
This recipe uses OpenAI `text-embedding-3-small` (1536 dims, cheap), but any
embeddings API works.

```python
from openai import OpenAI

embed_client = OpenAI(api_key=os.environ["OPENAI_API_KEY"])

def embed_batch(texts: list[str], model: str = "text-embedding-3-small") -> list[list[float]]:
    """Embed up to 2048 texts in one API call."""
    resp = embed_client.embeddings.create(input=texts, model=model)
    return [item.embedding for item in resp.data]

# Flatten: collect (chunk_text, source_url, chunk_index) for every page
records: list[dict] = []
for page in pages:
    source = page.get("metadata", {}).get("sourceURL", "")
    for chunk in page.get("chunks") or []:
        records.append({
            "text":   chunk["content"],
            "url":    source,
            "index":  chunk["index"],
        })

print(f"Total chunks to embed: {len(records)}")
# Total chunks to embed: 312

# Embed in batches of 200 to stay within API limits
BATCH = 200
vectors: list[list[float]] = []
for i in range(0, len(records), BATCH):
    batch_texts = [r["text"] for r in records[i : i + BATCH]]
    vectors.extend(embed_batch(batch_texts))

print(f"Embedded {len(vectors)} vectors (dim={len(vectors[0])})")
# Embedded 312 vectors (dim=1536)
```

---

## Step 4: Store in Chroma and query

```python
import chromadb

chroma = chromadb.Client()
collection = chroma.get_or_create_collection(
    name="pydantic_docs",
    metadata={"hnsw:space": "cosine"},
)

# Upsert all chunks — Chroma needs string IDs
collection.upsert(
    ids=[f"{r['url']}#{r['index']}" for r in records],
    embeddings=vectors,
    documents=[r["text"] for r in records],
    metadatas=[{"url": r["url"], "chunk_index": r["index"]} for r in records],
)

print(f"Stored {collection.count()} chunks in Chroma")
# Stored 312 chunks in Chroma

# --- Query ---
QUERY = "How do I define a field with a default value in Pydantic?"

q_vec = embed_batch([QUERY])[0]
results = collection.query(
    query_embeddings=[q_vec],
    n_results=3,
    include=["documents", "metadatas", "distances"],
)

for doc, meta, dist in zip(
    results["documents"][0],
    results["metadatas"][0],
    results["distances"][0],
):
    print(f"[score={1 - dist:.3f}] {meta['url']}")
    print(doc[:200])
    print()
```

**Expected query output:**

```
[score=0.871] https://docs.pydantic.dev/latest/concepts/fields/
## Default Values

Fields can have default values set either directly or via `Field(default=...)`:

```python
from pydantic import BaseModel, Field

class User(BaseModel):
    name: str
    age: int = 0          # direct default
    bio: str = Field(default="", description="Short bio")
```

[score=0.843] https://docs.pydantic.dev/latest/concepts/models/
## Field Definitions

Fields are defined as class attributes with type annotations. A field without a
default is required; a field with a default is optional...

[score=0.801] https://docs.pydantic.dev/latest/concepts/validators/
## Field Validators

Use `@field_validator` to add validation logic. Validators run after the default...
```

---

## pgvector alternative (Step 4b)

If you are already running Postgres, use pgvector instead of Chroma:

```python
import psycopg2
import json

conn = psycopg2.connect("postgresql://localhost/mydb")
cur  = conn.cursor()

cur.execute("CREATE EXTENSION IF NOT EXISTS vector")
cur.execute("""
    CREATE TABLE IF NOT EXISTS doc_chunks (
        id          TEXT PRIMARY KEY,
        url         TEXT,
        chunk_index INT,
        content     TEXT,
        embedding   vector(1536)
    )
""")

for rec, vec in zip(records, vectors):
    cur.execute(
        """
        INSERT INTO doc_chunks (id, url, chunk_index, content, embedding)
        VALUES (%s, %s, %s, %s, %s)
        ON CONFLICT (id) DO UPDATE SET embedding = EXCLUDED.embedding
        """,
        (f"{rec['url']}#{rec['index']}", rec["url"], rec["index"],
         rec["text"], json.dumps(vec)),
    )

conn.commit()

# Query
cur.execute(
    """
    SELECT url, content, 1 - (embedding <=> %s::vector) AS score
    FROM   doc_chunks
    ORDER BY embedding <=> %s::vector
    LIMIT  3
    """,
    (json.dumps(q_vec), json.dumps(q_vec)),
)

for url, content, score in cur.fetchall():
    print(f"[score={score:.3f}] {url}")
    print(content[:200])
    print()
```

---

## Complete script

```python
"""
recipe_rag.py — build a RAG knowledge base from a docs site with fastCRW.
Run: python recipe_rag.py
Requires: pip install crw chromadb openai
Env:      CRW_API_KEY, OPENAI_API_KEY
"""
import os
from crw import CrwClient
from openai import OpenAI
import chromadb

TARGET  = "https://docs.pydantic.dev/latest/"
MAX_PAGES = 50

client       = CrwClient(api_key=os.environ["CRW_API_KEY"])
embed_client = OpenAI(api_key=os.environ["OPENAI_API_KEY"])

# 1. Discover URLs
urls = client.map(TARGET, max_depth=3, use_sitemap=True)
doc_urls = [u for u in urls if "/api/" not in u and "#" not in u][:MAX_PAGES]
print(f"Discovered {len(doc_urls)} pages")

# 2. Scrape + chunk (POST /v1/scrape per URL — chunkStrategy is a v1 parameter)
pages = []
for url in doc_urls:
    page = client.scrape(
        url,
        formats=["markdown"],
        chunkStrategy={"type": "topic", "maxChars": 1500, "dedupe": True},
    )
    pages.append(page)
print(f"Scraped {len(pages)} pages")

# 3. Flatten chunks
records = [
    {"text": ch["content"], "url": pg.get("metadata", {}).get("sourceURL", ""), "index": ch["index"]}
    for pg in pages for ch in (pg.get("chunks") or [])
]
print(f"Total chunks: {len(records)}")

# 4. Embed
def embed_batch(texts: list[str]) -> list[list[float]]:
    resp = embed_client.embeddings.create(input=texts, model="text-embedding-3-small")
    return [item.embedding for item in resp.data]

BATCH   = 200
vectors = []
for i in range(0, len(records), BATCH):
    vectors.extend(embed_batch([r["text"] for r in records[i:i+BATCH]]))
print(f"Embedded {len(vectors)} vectors")

# 5. Store
chroma      = chromadb.Client()
collection  = chroma.get_or_create_collection("pydantic_docs", metadata={"hnsw:space": "cosine"})
collection.upsert(
    ids=[f"{r['url']}#{r['index']}" for r in records],
    embeddings=vectors,
    documents=[r["text"] for r in records],
    metadatas=[{"url": r["url"]} for r in records],
)
print(f"Stored {collection.count()} chunks")

# 6. Query
QUERY   = "How do I define a field with a default value in Pydantic?"
q_vec   = embed_batch([QUERY])[0]
results = collection.query(query_embeddings=[q_vec], n_results=3,
                           include=["documents", "metadatas", "distances"])
for doc, meta, dist in zip(results["documents"][0], results["metadatas"][0], results["distances"][0]):
    print(f"\n[score={1-dist:.3f}] {meta['url']}\n{doc[:300]}")
```

---

## Key parameters

| Parameter | Where | Effect |
|-----------|-------|--------|
| `chunkStrategy.type` | `POST /v1/scrape` only | `"topic"` splits on headings; `"sentence"` on `.!?`; `"regex"` on a custom pattern |
| `chunkStrategy.maxChars` | `POST /v1/scrape` only | Hard cap per chunk in characters |
| `chunkStrategy.dedupe` | `POST /v1/scrape` only | Drop near-duplicate chunks (Jaccard similarity > 85%) |
| `query` + `filterMode` | `POST /v1/scrape` only | Server-side BM25/cosine ranking — useful for one-shot QA; skip when indexing all chunks |
| `onlyMainContent` | `POST /v1/scrape`, `/firecrawl/v2/batch/scrape` | Strip nav/footer before chunking — strongly recommended |

> **`chunkStrategy` on batch/crawl:** `POST /firecrawl/v2/batch/scrape` and `POST /v1/crawl` do
> not forward `chunkStrategy` to the engine and never return a `chunks` field.
> Always use `client.scrape()` (looped) when you need per-page chunks, as shown above.

## Scaling tips

- **Large sites (> 500 pages):** loop `client.scrape()` with `asyncio.gather` or a
  thread pool to parallelize requests. Each call is a synchronous `POST /v1/scrape`
  that returns immediately with the result.
- **Embedding cost:** `text-embedding-3-small` costs ~$0.02 per 1M tokens. A 500-page
  docs site with 1500-char chunks produces roughly 400–600 chunks × ~300 tokens each =
  ~180 K tokens → under $0.01 total.
- **Re-indexing:** use the `changeTracking` format on re-scrapes to detect changed pages
  before re-embedding — avoids re-embedding unchanged content.
- **Persistent Chroma:** replace `chromadb.Client()` with
  `chromadb.PersistentClient(path="./chroma_db")` to survive restarts.
