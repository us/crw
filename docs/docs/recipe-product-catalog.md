# Recipe: Structured Product Catalog Extraction at Scale

Extract a full product catalog — name, price, SKU, availability, image URL — from
any e-commerce category page into a clean JSON list.

**Pipeline:**
```
category page  →  map product URLs  →  batch scrape with jsonSchema  →  aggregate JSON list
```

**Target site used in this recipe:** `https://books.toscrape.com/catalogue/category/books/mystery_3/`
— a public scraping sandbox with no auth, real pagination, and consistent product structure.

**Prerequisites:**

```bash
pip install crw
export CRW_API_KEY="crw-..."
# Also requires an LLM provider configured on the engine (or pass llmApiKey per request).
# The managed cloud (api.fastcrw.com) has one preconfigured.
```

---

## How extraction works

Structured extraction in CRW is a scrape mode, not a separate route.
Send `formats: ["json"]` and a `jsonSchema` to `POST /v1/scrape`.
The engine scrapes the page, converts it to markdown, passes that markdown to the
LLM with your schema, and returns validated JSON in `data.json`.

```
POST /v1/scrape
{
  "url": "...",
  "formats": ["json"],
  "jsonSchema": { <your JSON Schema> }
}
```

The `jsonSchema` field uses standard JSON Schema (`type`, `properties`, `required`,
`items`). You do not need to name every field on the page — only the ones you want back.

---

## Step 1: Map the category page to product URLs

Use `/v1/map` to collect all product URLs from the category page before scraping
any of them. This is faster and cheaper than crawling blind.

:::tabs
::tab{title="Python"}
```python
import os
from crw import CrwClient

client = CrwClient(api_key=os.environ["CRW_API_KEY"])

# Discover all URLs linked from the category page
all_urls = client.map(
    "https://books.toscrape.com/catalogue/category/books/mystery_3/",
    max_depth=1,        # stay on this category; don't follow the whole site
    use_sitemap=False,  # no sitemap for this sandbox — BFS only
)

# Keep only individual product pages
product_urls = [
    u for u in all_urls
    if "/catalogue/" in u and "/category/" not in u
]
print(f"Found {len(product_urls)} product URLs")
# Found 18 product URLs
```
::

::tab{title="cURL"}
```bash
curl -s -X POST https://api.fastcrw.com/v1/map \
  -H "Authorization: Bearer $CRW_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://books.toscrape.com/catalogue/category/books/mystery_3/",
    "maxDepth": 1,
    "useSitemap": false
  }' | python3 -c "
import sys, json
data = json.load(sys.stdin)
urls = [u for u in data.get('links', []) if '/catalogue/' in u and '/category/' not in u]
print(f'Found {len(urls)} product URLs')
for u in urls[:3]:
    print(' ', u)
"
```
::
:::

**Expected `/v1/map` response shape:**

```json
{
  "success": true,
  "links": [
    "https://books.toscrape.com/catalogue/sharp-objects_997/index.html",
    "https://books.toscrape.com/catalogue/in-a-dark-dark-wood_963/index.html",
    "https://books.toscrape.com/catalogue/the-past-never-ends_942/index.html"
  ]
}
```

> The Python SDK's `client.map()` returns the `links` list directly, not the full envelope.

---

## Step 2: Define the product schema

Design the smallest schema that captures what you need.
Start with required fields only; add optional fields once the required ones are stable.

```json
{
  "type": "object",
  "properties": {
    "title":        { "type": "string",  "description": "Full product name" },
    "price":        { "type": "string",  "description": "Price as displayed, e.g. '£12.99'" },
    "availability": { "type": "string",  "description": "Stock status, e.g. 'In stock'" },
    "rating":       { "type": "string",  "description": "Star rating word, e.g. 'Three'" },
    "description":  { "type": "string",  "description": "First paragraph of product description" },
    "image_url":    { "type": "string",  "description": "Absolute URL of the main product image" }
  },
  "required": ["title", "price", "availability"]
}
```

Keep `description` hints concise — they help the LLM find the right value without
inflating the prompt significantly.

---

## Step 3: Scrape each product URL with structured extraction

### Single product (verify schema first)

Before running at scale, verify the schema works on one URL.

:::tabs
::tab{title="Python"}
```python
PRODUCT_SCHEMA = {
    "type": "object",
    "properties": {
        "title":        {"type": "string",  "description": "Full product name"},
        "price":        {"type": "string",  "description": "Price as displayed, e.g. '£12.99'"},
        "availability": {"type": "string",  "description": "Stock status, e.g. 'In stock'"},
        "rating":       {"type": "string",  "description": "Star rating word, e.g. 'Three'"},
        "description":  {"type": "string",  "description": "First paragraph of product description"},
        "image_url":    {"type": "string",  "description": "Absolute URL of the main product image"},
    },
    "required": ["title", "price", "availability"],
}

result = client.scrape(
    "https://books.toscrape.com/catalogue/sharp-objects_997/index.html",
    formats=["json"],
    json_schema=PRODUCT_SCHEMA,
)

print(result["json"])
# {'title': 'Sharp Objects', 'price': '£47.82', 'availability': 'In stock',
#  'rating': 'Four', 'description': "WICKED above her hipbone..."}
```
::

::tab{title="cURL"}
```bash
curl -s -X POST https://api.fastcrw.com/v1/scrape \
  -H "Authorization: Bearer $CRW_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://books.toscrape.com/catalogue/sharp-objects_997/index.html",
    "formats": ["json"],
    "jsonSchema": {
      "type": "object",
      "properties": {
        "title":        {"type": "string",  "description": "Full product name"},
        "price":        {"type": "string",  "description": "Price as displayed, e.g. £12.99"},
        "availability": {"type": "string",  "description": "Stock status, e.g. In stock"},
        "rating":       {"type": "string",  "description": "Star rating word, e.g. Three"},
        "description":  {"type": "string",  "description": "First paragraph of product description"},
        "image_url":    {"type": "string",  "description": "Absolute URL of the main product image"}
      },
      "required": ["title", "price", "availability"]
    }
  }'
```
::
:::

**Expected single-product response:**

```json
{
  "success": true,
  "data": {
    "json": {
      "title": "Sharp Objects",
      "price": "£47.82",
      "availability": "In stock",
      "rating": "Four",
      "description": "WICKED above her hipbone, GIRL across her heart. Words are like a road map to reporter Camille Preaker's troubled past.",
      "image_url": "https://books.toscrape.com/media/cache/32/51/3251cf3a3412f53f339e42cac2134093.jpg"
    },
    "metadata": {
      "title": "Sharp Objects | Books to Scrape",
      "sourceURL": "https://books.toscrape.com/catalogue/sharp-objects_997/index.html",
      "statusCode": 200,
      "elapsedMs": 1240
    }
  }
}
```

The structured data lands in `data.json`. `data.metadata.sourceURL` is the canonical
link to attach to each product record.

---

## Step 4: Batch scrape all product URLs

Once the schema is verified, scrape all products in a single async batch job.
`batch_scrape` is HTTP mode only — it starts a job, polls until completion, and
returns a list of per-URL results.

> **How schema works in `/firecrawl/v2/batch/scrape`:** The batch endpoint parses its body
> into a `V2ScrapeRequest`, which has no top-level `jsonSchema` field — a bare
> `"jsonSchema": {...}` key is silently ignored by serde. The correct way to pass
> a schema is to **embed it inside the `formats` array** using the v2 object format:
> `{"type": "json", "schema": {...}}`. The engine's `decompose()` logic lifts the
> `schema` from that object and feeds it to the LLM extractor.

:::tabs
::tab{title="Python"}
```python
import json, os
from crw import CrwClient

client = CrwClient(api_key=os.environ["CRW_API_KEY"])

PRODUCT_SCHEMA = {
    "type": "object",
    "properties": {
        "title":        {"type": "string",  "description": "Full product name"},
        "price":        {"type": "string",  "description": "Price as displayed, e.g. '£12.99'"},
        "availability": {"type": "string",  "description": "Stock status, e.g. 'In stock'"},
        "rating":       {"type": "string",  "description": "Star rating word, e.g. 'Three'"},
        "description":  {"type": "string",  "description": "First paragraph of product description"},
        "image_url":    {"type": "string",  "description": "Absolute URL of the main product image"},
    },
    "required": ["title", "price", "availability"],
}

# --- Step 1: discover product URLs ---
all_urls = client.map(
    "https://books.toscrape.com/catalogue/category/books/mystery_3/",
    max_depth=1,
    use_sitemap=False,
)
product_urls = [
    u for u in all_urls
    if "/catalogue/" in u and "/category/" not in u
]
print(f"Discovered {len(product_urls)} products")

# --- Step 2: batch scrape — embed schema inside the formats object ---
# NOTE: Do NOT pass jsonSchema= as a separate kwarg. The /firecrawl/v2/batch/scrape
# handler has no top-level jsonSchema field; it is silently dropped.
# Instead, use the v2 object format: {"type": "json", "schema": <schema>}.
pages = client.batch_scrape(
    urls=product_urls,
    formats=[{"type": "json", "schema": PRODUCT_SCHEMA}],
    onlyMainContent=True,
    poll_interval=3.0,
    timeout=300.0,
)

# --- Step 3: aggregate into a clean catalog list ---
catalog: list[dict] = []
for page in pages:
    product = page.get("json")
    if not product:
        continue
    product["source_url"] = page.get("metadata", {}).get("sourceURL", "")
    catalog.append(product)

print(f"Extracted {len(catalog)} products")
print(json.dumps(catalog[:2], indent=2, ensure_ascii=False))
```
::

::tab{title="cURL — start job"}
```bash
# Step 4a: start the batch scrape job
# Schema goes inside the formats array as {"type":"json","schema":{...}},
# NOT as a top-level "jsonSchema" field (which is silently ignored by the server).
curl -s -X POST https://api.fastcrw.com/firecrawl/v2/batch/scrape \
  -H "Authorization: Bearer $CRW_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "urls": [
      "https://books.toscrape.com/catalogue/sharp-objects_997/index.html",
      "https://books.toscrape.com/catalogue/in-a-dark-dark-wood_963/index.html",
      "https://books.toscrape.com/catalogue/the-past-never-ends_942/index.html"
    ],
    "formats": [
      {
        "type": "json",
        "schema": {
          "type": "object",
          "properties": {
            "title":        {"type": "string",  "description": "Full product name"},
            "price":        {"type": "string",  "description": "Price as displayed, e.g. £12.99"},
            "availability": {"type": "string",  "description": "Stock status, e.g. In stock"},
            "rating":       {"type": "string",  "description": "Star rating word, e.g. Three"},
            "description":  {"type": "string",  "description": "First paragraph of product description"},
            "image_url":    {"type": "string",  "description": "Absolute URL of the main product image"}
          },
          "required": ["title", "price", "availability"]
        }
      }
    ],
    "onlyMainContent": true
  }'
# { "success": true, "id": "7c9e6679-7425-40de-944b-e07fc1f90ae7", ... }
```
::

::tab{title="cURL — poll and collect"}
```bash
# Step 4b: poll until status == "completed"
JOB_ID="7c9e6679-7425-40de-944b-e07fc1f90ae7"

curl -s "https://api.fastcrw.com/firecrawl/v2/batch/scrape/$JOB_ID" \
  -H "Authorization: Bearer $CRW_API_KEY" \
  | python3 -c "
import sys, json
body = json.load(sys.stdin)
print('status:', body.get('status'))
for page in body.get('data', []):
    product = page.get('json', {})
    url = page.get('metadata', {}).get('sourceURL', '')
    if product:
        print(product.get('title'), '|', product.get('price'), '|', url)
"
```
::
:::

---

## Sample aggregated result

```json
[
  {
    "title": "Sharp Objects",
    "price": "£47.82",
    "availability": "In stock",
    "rating": "Four",
    "description": "WICKED above her hipbone, GIRL across her heart. Words are like a road map to reporter Camille Preaker's troubled past.",
    "image_url": "https://books.toscrape.com/media/cache/32/51/3251cf3a3412f53f339e42cac2134093.jpg",
    "source_url": "https://books.toscrape.com/catalogue/sharp-objects_997/index.html"
  },
  {
    "title": "In a Dark, Dark Wood",
    "price": "£19.63",
    "availability": "In stock",
    "rating": "One",
    "description": "Leonora, whose life in London is quiet and reclusive, receives an out-of-the-blue invitation to a hen do in a remote glass house in the woods.",
    "image_url": "https://books.toscrape.com/media/cache/a4/23/a423e30e3bc9cac1db16ccf0e87cf62f.jpg",
    "source_url": "https://books.toscrape.com/catalogue/in-a-dark-dark-wood_963/index.html"
  },
  {
    "title": "The Past Never Ends",
    "price": "£56.50",
    "availability": "In stock",
    "rating": "Four",
    "description": "Jackson Brooks has spent his entire career documenting the lives of people on the margins of society.",
    "image_url": "https://books.toscrape.com/media/cache/09/a3/09a3aef48557576e1a85ba7efea8ecb1.jpg",
    "source_url": "https://books.toscrape.com/catalogue/the-past-never-ends_942/index.html"
  }
]
```

---

## Complete script

```python
"""
recipe_product_catalog.py — extract a product catalog with fastCRW.

Run:
    pip install crw
    export CRW_API_KEY="crw-..."
    python recipe_product_catalog.py

Output: catalog.json (one object per product)
"""
import json
import os
from crw import CrwClient

CATEGORY_URL = "https://books.toscrape.com/catalogue/category/books/mystery_3/"

PRODUCT_SCHEMA = {
    "type": "object",
    "properties": {
        "title":        {"type": "string",  "description": "Full product name"},
        "price":        {"type": "string",  "description": "Price as displayed, e.g. '£12.99'"},
        "availability": {"type": "string",  "description": "Stock status, e.g. 'In stock'"},
        "rating":       {"type": "string",  "description": "Star rating word, e.g. 'Three'"},
        "description":  {"type": "string",  "description": "First paragraph of product description"},
        "image_url":    {"type": "string",  "description": "Absolute URL of the main product image"},
    },
    "required": ["title", "price", "availability"],
}

client = CrwClient(api_key=os.environ["CRW_API_KEY"])

# 1. Map category → product URLs
all_urls = client.map(CATEGORY_URL, max_depth=1, use_sitemap=False)
product_urls = [u for u in all_urls if "/catalogue/" in u and "/category/" not in u]
print(f"Discovered {len(product_urls)} products")

# 2. Batch scrape — embed schema inside the formats object (v2 format)
# A top-level jsonSchema= kwarg is NOT supported by /firecrawl/v2/batch/scrape and
# would be silently dropped. Use {"type":"json","schema":<schema>} instead.
pages = client.batch_scrape(
    urls=product_urls,
    formats=[{"type": "json", "schema": PRODUCT_SCHEMA}],
    onlyMainContent=True,
    poll_interval=3.0,
    timeout=300.0,
)

# 3. Aggregate — drop pages where extraction returned nothing
catalog: list[dict] = []
for page in pages:
    product = page.get("json")
    if not product:
        continue
    product["source_url"] = page.get("metadata", {}).get("sourceURL", "")
    catalog.append(product)

print(f"Extracted {len(catalog)} / {len(product_urls)} products successfully")

# 4. Save
with open("catalog.json", "w", encoding="utf-8") as f:
    json.dump(catalog, f, indent=2, ensure_ascii=False)

print("Saved catalog.json")
```

---

## Key parameters

| Parameter | Wire name | Where | Effect |
|-----------|-----------|-------|--------|
| `formats=["json"]` | `formats` | `/v1/scrape` | Activates structured extraction on the v1 endpoint; pair with `jsonSchema` |
| `formats=[{"type":"json","schema":{...}}]` | `formats` | `/firecrawl/v2/batch/scrape` | v2 object format — embeds the schema inside the formats array entry; the only correct way to pass a schema to the batch endpoint |
| `json_schema=...` | `jsonSchema` | `/v1/scrape` (Python SDK) | Named param; auto-adds `"json"` to formats; sends `jsonSchema` as a top-level field (valid only on `/v1/scrape`) |
| `only_main_content=True` | `onlyMainContent` | both | Strip nav/footer before extraction — strongly recommended |
| `max_depth=1` | `maxDepth` | `/v1/map` | Stay on the category page; set higher to follow pagination |

> **Important — schema delivery differs between v1 and v2 batch:**
> `client.scrape()` sends `jsonSchema` as a top-level field to `/v1/scrape`, where
> `ScrapeRequest` has that field. `/firecrawl/v2/batch/scrape` parses the body into
> `V2ScrapeRequest`, which has **no** top-level `jsonSchema` field — a bare
> `"jsonSchema": {...}` key is silently ignored by serde. Always embed the schema
> inside the `formats` array using the object form shown above for batch jobs.

---

## Schema design tips

- **Start small.** Three required fields (`title`, `price`, `availability`) is a
  better starting point than twenty optional ones. Expand only after the first
  result looks correct.
- **Use `description` hints.** When field names are ambiguous (e.g. `rating` could
  be numeric or a word), a short hint helps the LLM pick the right value.
- **Verify on one URL first.** Run `client.scrape()` on a single product page before
  committing to a batch job. LLM extraction errors are easier to debug one at a time.
- **Check `data.json` is not `null`.** The engine returns `null` for `json` if the
  LLM could not fill any required fields. Filter these out and inspect the
  `data.metadata.sourceURL` to see which pages failed.

---

## Scaling tips

- **Large catalogs (> 100 pages):** The `/firecrawl/v2/batch/scrape/{id}` poll response
  paginates results via a `next` cursor when results exceed the page limit —
  **the SDK does not follow the cursor; it returns only the first page (up to
  100 URLs)**. For catalogs with more than 100 product URLs, use the raw-HTTP
  pagination loop shown in the [recipe-batch.md](./recipe-batch.md) recipe.
- **Multiple categories:** call `client.map()` per category, union the URL lists,
  deduplicate, then pass everything to a single `batch_scrape`.
- **Re-scraping for updates:** use `formats: ["json", "changeTracking"]` on
  re-runs. Pages that have not changed come back with `changeTracking.status:
  "unchanged"` — skip their re-extraction to save LLM credits.
- **LLM cost:** structured extraction consumes LLM tokens. Use `onlyMainContent:
  true` to reduce input size. A typical product page costs 200–600 input tokens.
