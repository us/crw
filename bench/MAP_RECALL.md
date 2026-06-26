# /map recall benchmark

Turns "map keeps missing URLs" into a number. Measures how completely `/map`
discovers a site's URLs against ground truth we control.

## Why

There is no off-the-shelf benchmark for "given a site, find all its URLs."
So we use **purpose-built scraper sandboxes** with fixed, publicly-known
structure as ground truth — deterministic, unprotected, free:

| Site | Known ground truth |
|------|--------------------|
| books.toscrape.com | 1000 products, 50 listing pages, 50 categories |
| quotes.toscrape.com | 10 pages of quotes |

The metric that matters is **recall**. A raw URL count lies: `trendyol.com`
returns ~5082 URLs but ~0 of the Turkish catalog — almost all are `/ar/` + `/bg/`
foreign-locale category sitemaps (20 of 5082 are product pages). Count != recall.

## Run

Against a local crw server:

```bash
CRW_API_URL=http://localhost:3000 uv run python bench/map_recall.py
```

Offline scorer self-check (no server, uses a recorded fixture):

```bash
uv run python bench/map_recall.py --selfcheck
```

## Baseline findings (hosted fastCRW, 2026-06-26, default maxDepth=2)

- **books.toscrape.com**: products **709/1000 (recall 0.71)**; main-catalogue
  pagination **4/50** pages reached. Clean, unprotected, sitemap-less static
  site still loses ~29% of products. Recall is carried by category pages
  (fully found); deep `page-N` pagination dies at the depth wall.
- **`maxDepth` 5 and 10 → HTTP 502** on hosted (default depth 2 works). Can't
  raise depth to compensate.
- **trendyol.com**: 5082 URLs, ~0 TR content, 20 products — count != recall;
  bot-protected.

## Root cause (for follow-up, not fixed here)

`crw-crawl/src/crawl.rs::discover_urls` runs a BFS gated by `depth < max_depth`
(default 2). Pagination is a *chain* (`page-2 → page-3 → …`), so depth-2 reaches
only ~`page-3`; everything deeper is lost. The fix is pagination-aware
traversal (follow `rel=next` / numbered pages on a separate budget) rather than
just raising the default depth — raising depth lengthens the crawl and worsens
the gateway 502. Tracked separately.
