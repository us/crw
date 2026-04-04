# Changelog

## 2026-04-02 -- Search API

### New: Search Endpoint

- `POST /api/v1/search` (cloud) / `POST /v1/search` (self-hosted) -- search the web with optional content scraping
- Supports web, news, and image results via `sources` parameter
- Time-based filtering with `tbs` parameter (`qdr:h`, `qdr:d`, `qdr:w`, `qdr:m`, `qdr:y`)
- Category filtering: `github`, `research`, `pdf`
- Optional `scrapeOptions` to scrape each result URL in one call
- 1 credit per search + 1 per scraped result (failed scrapes refunded). Cloud only (fastcrw.com).
- Grouped response format when `sources` is specified

## 2026-03-11 -- Engine v0.0.8

This release focused on two themes:

- making extraction behavior more reliable on real-world content,
- and making the product surface easier to understand through clearer docs and validation.

### Engine (CRW)

- **Wikipedia / MediaWiki onlyMainContent fix** -- `onlyMainContent: true` now correctly extracts article text from Wikipedia pages (~49% size reduction). Previously the noise handler matched `"toc"` as a substring inside `"vector-toc-available"` on the `<html>` element, removing the entire page.
- **3-tier noise pattern matching** -- noise class/id matching now uses substring (long patterns), exact-token (short/ambiguous: `toc`, `share`, `social`, `comment`, `related`), and prefix (`ad-`, `ads-`) matching to avoid false positives on real content.
- **Structural element guard** -- noise handler never removes `<html>`, `<head>`, `<body>`, or `<main>` elements.
- **Re-clean after readability** -- readability output is re-cleaned to strip residual noise (infobox, navbox, catlinks) inside broad containers.
- **Wikipedia-aware readability** -- added `.mw-parser-output`, `#mw-content-text`, `#bodyContent` to scored selectors; selectors wrapping >90% of body are skipped.
- **BYOK LLM extraction** -- per-request `llmApiKey`, `llmProvider`, `llmModel` fields for bring-your-own-key structured extraction without server config.
- **JSON format validation** -- `formats: ["json"]` without `jsonSchema` now returns a 400 error instead of a warning.
- **Block detection skip** -- pages >50 KB skip interstitial/block detection (no more false "blocked by anti-bot" on Wikipedia).
- **Null byte protection** -- URLs containing `%00` or null bytes are rejected at the validation layer.
- **Request timeout** -- default bumped from 60s to 120s.
- **Dockerfile fix** -- corrected `cargo build` flags, added `config.docker.toml`.

### Upgrade notes

- Re-test any extraction workflow that depends on Wikipedia or MediaWiki-style content because `onlyMainContent` behavior is now more aggressive and more accurate.
- If you were relying on permissive `json` requests without a schema, update the client now; those requests return a 400 error in this release.
- If you self-host, pull the latest container image so the Dockerfile and config changes land together.

---

## 2026-03-10 -- Initial Release

- Scrape, crawl, and map endpoints -- Firecrawl-compatible API shape.
- Markdown-first extraction with readability scoring.
- CSS/XPath selectors, tag include/exclude filtering.
- BM25 and cosine similarity chunk filtering.
- LLM-based structured extraction with JSON Schema validation.
- JS rendering via LightPanda CDP.
- Stealth mode with browser-realistic UA rotation.
- Self-hosting support with single-binary deployment.

### Release framing

The first release established the core product surface: a Firecrawl-compatible scrape, crawl, and map API with markdown-first extraction, optional browser rendering, and a path to self-hosting. Later releases should be read as refinements on top of that baseline, not a new product direction.
