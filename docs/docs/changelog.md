# Changelog

This page is generated from the root [`CHANGELOG.md`](https://github.com/us/crw/blob/main/CHANGELOG.md), which is maintained by release-please during releases.

:::note
The source of truth is the repository root changelog. Do not edit this docs page manually.
:::

All notable changes to CRW are documented here.

## [0.3.4](https://github.com/us/crw/compare/v0.3.3...v0.3.4) (2026-04-09)


### Bug Fixes

* **config:** env var overrides not applied due to missing prefix_separator ([71c7ae5](https://github.com/us/crw/commit/71c7ae544bd74575cf30101190d65708934bcb1f)), closes [#18](https://github.com/us/crw/issues/18)

## [0.3.3](https://github.com/us/crw/compare/v0.3.2...v0.3.3) (2026-04-09)


### Features

* add APT/Debian package distribution ([c34b8e9](https://github.com/us/crw/commit/c34b8e93563c0f3f4b3fa1a1e395e6041e97766c))
* **renderer:** spawn all available browsers for multi-renderer fallback ([f546437](https://github.com/us/crw/commit/f546437568eb833684978793e5b79880f2710016))

## [0.3.2](https://github.com/us/crw/compare/v0.3.1...v0.3.2) (2026-04-08)


### Bug Fixes

* **cli:** auto-prepend https:// when no scheme provided ([1050606](https://github.com/us/crw/commit/105060644acb1ef930555869372f5413947ac194))

## [0.3.1](https://github.com/us/crw/compare/v0.3.0...v0.3.1) (2026-04-08)


### Features

* add llms.txt, SKILL.md, MCP init command, and docs UI improvements ([1b22d19](https://github.com/us/crw/commit/1b22d194ec9a8b8db8db39a1d44ce38a56716f5d))
* add one-line install script with auto platform detection ([6354f79](https://github.com/us/crw/commit/6354f7929896bb9a12eeebb9d5bf439731fac874))
* **docs:** add dark mode logo support and improve docs UI ([047df7b](https://github.com/us/crw/commit/047df7b46485d9cdec76b213db181dc0aa592511))
* **docs:** align design with SaaS site and update branding ([631d07c](https://github.com/us/crw/commit/631d07c66a34bcc71e8d2494a177182977886a8c))
* **docs:** unify docs into docs.fastcrw.com with Mintlify-style design ([4994998](https://github.com/us/crw/commit/4994998ae97924100e4415b90a5b78db3b8cf09e))
* **docs:** update URLs, dark mode, syntax highlighting, and benchmarks ([0678cdf](https://github.com/us/crw/commit/0678cdfde6a9de9b5fa56ec2c6256a63a0342317))
* release all 3 binaries, CLI auto-browser, README overhaul ([aa2950d](https://github.com/us/crw/commit/aa2950d564cb97f668c0c838713f612bffa33e32))
* update README banner with new logo ([bcba1ad](https://github.com/us/crw/commit/bcba1ad95e7ac1afd3066827f2327452b60c81f0))


### Bug Fixes

* crawl HTTP polling bug + SDK test suite + docs ([#16](https://github.com/us/crw/issues/16)) ([b6d8983](https://github.com/us/crw/commit/b6d898360cb2396683e9e82f778d2ee3b8455625))
* remove internal implementation detail from roadmap ([a5013f0](https://github.com/us/crw/commit/a5013f07f039615de5c5988e6bbf64629b56aa0e))

## [0.3.0](https://github.com/us/crw/compare/v0.2.2...v0.3.0) (2026-04-02)


### Features

* add search() method to Python SDK and docs ([591e3fe](https://github.com/us/crw/commit/591e3fed4bbd14b4470fd2f5cbc24c02f7543dba))

## [0.2.2](https://github.com/us/crw/compare/v0.2.1...v0.2.2) (2026-04-02)


### Bug Fixes

* **renderer:** escalate to JS renderer on HTTP 401/403 responses ([f515caa](https://github.com/us/crw/commit/f515caa2e315a06df9274967bdc3fb23dbafcbcf))
* use GitHub latest release instead of pinned version for binary download ([4afcb1a](https://github.com/us/crw/commit/4afcb1a4c67d4e9b36809ed32ce88b6a9fd4c342))

## [0.2.1](https://github.com/us/crw/compare/v0.2.0...v0.2.1) (2026-03-28)


### Bug Fixes

* make crw-mcp npm wrapper executable ([576a9eb](https://github.com/us/crw/commit/576a9eb19ae90bc677344045dd70fb96e8b938da))
* use latest tag in server.json OCI identifier ([7ec3b82](https://github.com/us/crw/commit/7ec3b82ab78aa2c7ff900d07400f8a2426cb955f))

## [0.2.0](https://github.com/us/crw/compare/v0.1.2...v0.2.0) (2026-03-28)


### Features

* add MCP Registry support for official server discovery ([154b9f5](https://github.com/us/crw/commit/154b9f520015260d012ebf899513c3af1f9dfe3d))

## [0.1.2](https://github.com/us/crw/compare/v0.1.1...v0.1.2) (2026-03-27)


### Bug Fixes

* vendor pdf-inspector as crw-pdf for crates.io publishability ([3f7681d](https://github.com/us/crw/commit/3f7681dde0b78ff2c0c11d232d21a714edb94d75))

## [0.1.1](https://github.com/us/crw/compare/v0.1.0...v0.1.1) (2026-03-26)


### Bug Fixes

* skip already-published crates without masking real errors ([010649c](https://github.com/us/crw/commit/010649c522e4e49edcf51873d4c1065e08d510b1))

## [0.1.0](https://github.com/us/crw/compare/v0.0.14...v0.1.0) (2026-03-26)


### Features

* add PDF extraction support via pdf-inspector ([06dd5bf](https://github.com/us/crw/commit/06dd5bf89caf004929f22d41d00f8a297d09b825))

## [0.0.14](https://github.com/us/crw/compare/v0.0.13...v0.0.14) (2026-03-25)


### Features

* **mcp:** auto-download LightPanda binary for zero-config JS rendering ([41f443b](https://github.com/us/crw/commit/41f443b885326401b653cfcba0054cb943672ca6))
* **mcp:** auto-spawn headless Chrome for JS rendering in embedded mode ([9a6b0ae](https://github.com/us/crw/commit/9a6b0ae3f16399f8f9f233109e431f74a882d973))


### Bug Fixes

* **ci:** move crw-mcp to Tier 4 in release workflow and add workflow_dispatch ([d7584a8](https://github.com/us/crw/commit/d7584a82c0dd4ac0c6cd8b169b19939d92eb4e95))

## [0.0.13](https://github.com/us/crw/compare/v0.0.12...v0.0.13) (2026-03-24)


### Features

* **mcp:** add embedded mode — self-contained MCP server, no crw-server needed ([75e5450](https://github.com/us/crw/commit/75e54504487f24ee30c0272bb83eb9aab807a284))


### Bug Fixes

* **ci:** switch release-please to simple type for Rust workspace support ([51cd420](https://github.com/us/crw/commit/51cd420ab77e4bd58bf1a6a7ab0c28287896a0b7))

## v0.0.12

- **Readability drill-down** — when `<main>` or `<article>` wraps >90% of body, the extractor now searches inside for narrower content elements (`.main-page-content`, `.article-content`, `.entry-content`, etc.) instead of discarding. Fixes MDN pages returning 35 chars and StackOverflow returning only the question
- **Base64 image stripping** — `data:` URI images are removed in both HTML cleaning (lol_html) and markdown post-processing (regex safety net). Eliminates massive base64 blobs from Reddit and similar sites
- **Select/dropdown removal** — `<select>` elements removed in `onlyMainContent` mode; dropdown/city-selector/location-selector noise patterns added. Fixes Hürriyet city dropdown leaking into content
- **Extended scored selectors** — added `.main-page-content`, `.js-post-body`, `.s-prose`, `#question`, `.page-content`, `#page-content`, `[role="article"]` for better MDN, StackOverflow, and generic site coverage
- **Smarter fallback chain** — when primary extraction produces too-short markdown, both fallbacks (cleaned HTML and basic clean) are tried and the longer result is picked, instead of short-circuiting on non-empty but insufficient content

## v0.0.11

- **Stealth anti-bot bypass** — automatic stealth JS injection via `Page.addScriptToEvaluateOnNewDocument` before every CDP navigation. Spoofs `navigator.webdriver`, Chrome runtime object, plugins array, languages, permissions API, iframe `contentWindow`, and `toString()` proxy to bypass Cloudflare, PerimeterX, and other bot detection systems
- **Cloudflare challenge auto-retry** — detects Cloudflare JS challenge pages ("Just a moment", `cf-browser-verification`, `challenge-platform`) after page load and polls up to 3 times at 3-second intervals for non-interactive challenges to auto-resolve
- **HTTP → CDP auto-escalation** — `FallbackRenderer::fetch()` in auto mode now checks HTTP responses for anti-bot challenge signatures and automatically escalates to JS rendering when detected, instead of returning the challenge HTML
- **Chrome failover in Docker** — full automatic failover chain: HTTP → LightPanda → Chrome. Added `chromedp/headless-shell` as a Docker Compose sidecar service with 2GB shared memory. If LightPanda crashes on complex SPAs (React, Angular), Chrome handles the render
- **Chrome WS URL auto-discovery** — CDP renderer resolves Chrome DevTools WebSocket URL via the `/json/version` HTTP endpoint with `Host: localhost` header (required for chromedp/headless-shell's socat proxy). Uses `OnceCell` for lazy one-time resolution
- **Proxy configuration docs** — expanded proxy config comments with examples for HTTP, SOCKS5, and residential proxy providers (IPRoyal, Oxylabs, Smartproxy)
- **Raw string delimiter fix** — fixed `markdown.rs` test that used `r#"..."#` with a string containing `"#`, changed to `r##"..."##`

## v0.0.10 / v0.0.9

- **Crawl cancel endpoint** — `DELETE /v1/crawl/{id}` cancels a running crawl job via `AbortHandle` and returns `{ success: true }`
- **API rate limiting** — token-bucket rate limiter (configurable `rate_limit_rps`, default 10). Returns 429 with `error_code: "rate_limited"` when exceeded
- **Machine-readable error codes** — all error responses now include an `error_code` field (e.g. `"invalid_url"`, `"http_error"`, `"rate_limited"`, `"not_found"`)
- **Map response envelope** — `/v1/map` now returns `{ success, data: { links } }` instead of `{ success, links }` for consistency with other endpoints
- **Fenced code blocks** — indented code blocks (4-space) are post-processed into fenced (```) blocks for better LLM/RAG compatibility
- **Sphinx footer cleanup** — `"footer"` added to exact-token noise patterns, catching `<div class="footer">` in Sphinx/documentation sites
- **`renderedWith: "http"`** — HTTP-only fetches now report `rendered_with: "http"` in metadata instead of `null`
- **405 JSON responses** — all routes now have `.fallback(method_not_allowed)` returning structured JSON with `error_code: "method_not_allowed"` instead of empty bodies
- **Anchor link cleanup** — empty anchor links (`[](#id)`, `[¶](#id)`) and pilcrow/section signs stripped from Markdown output
- **`role="contentinfo"` cleanup** — elements with ARIA roles `contentinfo`, `navigation`, `banner`, `complementary` removed during cleaning
- **Tiny chunk merging** — topic chunking merges heading-only chunks (<50 chars) with the next chunk to improve RAG embedding quality

## v0.0.8

- **Wikipedia / MediaWiki onlyMainContent fix** — `onlyMainContent: true` now correctly extracts article text from Wikipedia pages (~49% size reduction). Previously the `<html>` element's `class="vector-toc-available"` matched the `"toc"` noise pattern via substring, removing the entire page
- **3-tier noise pattern matching** — noise class/id matching now uses substring (long patterns), exact-token (short/ambiguous: `toc`, `share`, `social`, `comment`, `related`), and prefix (`ad-`, `ads-`) matching to avoid false positives
- **Structural element guard** — noise handler never removes `<html>`, `<head>`, `<body>`, or `<main>` elements
- **Re-clean after readability** — readability output is re-cleaned to strip residual noise (infobox, navbox, catlinks) that survives inside broad containers
- **Wikipedia-aware readability** — added `.mw-parser-output`, `#mw-content-text`, `#bodyContent` to scored selectors; priority/scored selectors that wrap >90% of body are skipped
- **BYOK LLM extraction** — per-request `llmApiKey`, `llmProvider`, `llmModel` fields for bring-your-own-key structured extraction without server config
- **JSON format validation** — `formats: ["json"]` without `jsonSchema` now returns a 400 error instead of a warning
- **Block detection skip** — pages >50 KB skip interstitial/block detection (no more false "blocked by anti-bot" on Wikipedia)
- **Null byte URL rejection** — URLs with `%00` or null bytes rejected at validation
- **Request timeout** — default timeout bumped from 60s to 120s
- **Dockerfile fix** — corrected `cargo build` flags, added `config.docker.toml`

## v0.0.7

- **`success: false` on 4xx targets** — scraping a 403/404/429 target with minimal body now correctly returns `success: false` with error details, instead of `success: true` with a warning. Targets with real content (custom error pages) still return `success: true` with a warning
- **JS renderer fallback warning** — when `renderJs: true` is requested but no CDP renderer is available, the response now includes `rendered_with: "http_only_fallback"` and a warning instead of silently falling back
- **CDP health check** — `is_available()` now runs a real `Browser.getVersion` command instead of just testing the WebSocket connection
- **Specific error messages** — unknown formats now return descriptive errors (e.g., `"Unknown format 'extract'. Valid formats: ..."`) instead of generic 422
- **`"extract"` format alias** — `formats: ["extract"]` and `formats: ["llm-extract"]` are now accepted as aliases for `"json"` (Firecrawl compatibility)
- **Chunk dedup by default** — deduplication is now enabled by default for all chunking strategies; separator-only chunks (`---`, `***`) are filtered out
- **Chunk relevance scores** — chunks now return `{ content, score, index }` objects instead of plain strings when a query is provided
- **Map timeout** — `/v1/map` accepts a `timeout` parameter (default 120s, max 300s) to prevent 502s on large sites
- **Stealth + JS rendering fix** — `stealth: true` with `renderJs: true` no longer bypasses CDP; the shared renderer is used with stealth headers injected
- **BM25 NaN guard** — prevents `NaN` scores when all chunks are empty

## v0.0.6

- **Crate READMEs on crates.io** — all 7 crates now have detailed README documentation visible on their crates.io pages, with usage examples, API docs, and installation instructions

## v0.0.5

- **`crw-cli` now on crates.io** — install the standalone CLI with `cargo install crw-cli` and scrape URLs without running a server
- **Parallelized release workflow** — crate publishing uses tiered parallelism, cutting release time by ~2.25 minutes
- **CLI and MCP install docs** — README now includes `cargo install` instructions for both `crw-cli` and `crw-mcp`

## v0.0.4

- **Hardened rendering and warning semantics** — improved reliability of the rendering pipeline and warning detection logic
- **XPath output escaping** — XPath extraction results are now properly escaped to prevent injection
- **Broadened status warnings** — expanded HTTP status code range that triggers warning metadata
- **Capped interstitial scan** — bounded interstitial page detection to avoid excessive scanning
- **Clippy cleanup** — simplified status code checks for cleaner, idiomatic Rust

## v0.0.3

- **Warning-aware target handling** — 4xx and anti-bot targets now return `success: true` with `warning` and `metadata.statusCode`
- **More reliable JS rendering** — CDP navigation now waits for real page lifecycle completion before applying `waitFor`
- **Stealth decompression fix** — gzip and brotli responses decode cleanly instead of leaking garbled binary payloads
- **Crawl compatibility** — `limit`, `maxPages`, and `max_pages` now normalize to the same crawl cap
- **XPath and chunking fixes** — XPath returns all matches, chunk overlap/dedupe is supported, and scorer rank order is preserved

## v0.0.2

- **CSS selector & XPath** — target specific DOM elements before Markdown conversion (`cssSelector`, `xpath`)
- **Chunking strategies** — split content into topic, sentence, or regex-delimited chunks for RAG pipelines (`chunkStrategy`)
- **BM25 & cosine filtering** — rank chunks by relevance to a query and return top-K results (`filterMode`, `topK`)
- **Better Markdown** — switched to `htmd` (Turndown.js port): tables, code block languages, nested lists all render correctly
- **Stealth mode** — rotate User-Agent from a built-in Chrome/Firefox/Safari pool and inject 12 browser-like headers (`stealth: true`)
- **Per-request proxy** — override the global proxy on a per-request basis (`proxy: "http://..."`)
- **Rate limit jitter** — randomized delay between requests to avoid uniform traffic fingerprinting
- **`crw-server setup`** — one-command JS rendering setup: downloads LightPanda, creates `config.local.toml`

## v0.0.1

- **Firecrawl-compatible REST API** — `/v1/scrape`, `/v1/crawl`, `/v1/map` with identical request/response format
- **6 output formats** — markdown, HTML, cleaned HTML, raw HTML, plain text, links, structured JSON
- **LLM structured extraction** — JSON schema in, validated structured data out (Anthropic tool_use + OpenAI function calling)
- **JS rendering** — auto-detect SPAs via heuristics, render via LightPanda, Playwright, or Chrome (CDP)
- **BFS crawler** — async crawl with rate limiting, robots.txt, sitemap support, concurrent jobs
- **MCP server** — built-in stdio + HTTP transport for Claude Code and Claude Desktop
- **SSRF protection** — private IPs, cloud metadata, IPv6, dangerous URI filtering
- **Docker ready** — multi-stage build with LightPanda sidecar
