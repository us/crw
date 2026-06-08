"""Deterministic request corpus for the Firecrawl v2 conformance suite.

Both `capture` (vs the real api.firecrawl.dev) and `compare` (vs a local crw)
drive the SAME requests so the responses are diffable. Async endpoints
(crawl/batch/extract) are start + poll-to-terminal.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

# A fixed, low-cost target set. example.com is a stable static page; the others
# exercise a JS-rendered site and a richer sitemap.
EXAMPLE = "https://example.com"
RICH = "https://firecrawl.dev"
# A small, stable public PDF for exercising the `parsers` option end-to-end.
PDF_URL = "https://www.w3.org/WAI/ER/tests/xhtml/testfiles/resources/pdf/dummy.pdf"


@dataclass(frozen=True)
class Case:
    """One conformance case.

    kind: "sync" (POST returns the document) or "job" (POST returns an id, then
    poll `status_path_tmpl`.format(id=...) until status in terminal states).

    upload_file: when set, the request is sent as multipart/form-data with a
    `file` part (bytes of this path, relative to the repo root) and an `options`
    part (JSON of `body`). Used for `POST /v2/parse`.
    """

    name: str
    method: str
    path: str
    body: dict[str, Any] = field(default_factory=dict)
    kind: str = "sync"
    status_path_tmpl: str | None = None
    tier: int = 1
    upload_file: str | None = None


# ── Scrape format matrix (the headline v1→v2 delta) ──
SCRAPE_CASES: list[Case] = [
    Case("scrape_markdown_string", "POST", "/v2/scrape",
         {"url": EXAMPLE, "formats": ["markdown"]}),
    Case("scrape_markdown_object", "POST", "/v2/scrape",
         {"url": EXAMPLE, "formats": [{"type": "markdown"}]}),
    Case("scrape_html_rawhtml_links", "POST", "/v2/scrape",
         {"url": EXAMPLE, "formats": ["html", "rawHtml", "links"]}),
    Case("scrape_json_schema", "POST", "/v2/scrape",
         {"url": EXAMPLE, "formats": [
             {"type": "json",
              "schema": {"type": "object",
                         "properties": {"title": {"type": "string"}}}}]}),
    Case("scrape_summary", "POST", "/v2/scrape",
         {"url": EXAMPLE, "formats": [{"type": "summary"}]}, tier=2),
    Case("scrape_multi_format", "POST", "/v2/scrape",
         {"url": EXAMPLE, "formats": [
             "markdown", "links",
             {"type": "json", "schema": {"type": "object"}}]}),
    # `parsers` option (PDF). String form, object form (+maxPages), and disable.
    # `maxAge: 0` forces a fresh fetch so Firecrawl never serves from cache —
    # keeps the golden deterministic (no `cachedAt`/`cacheState:hit` artifacts).
    Case("scrape_pdf_parsers_string", "POST", "/v2/scrape",
         {"url": PDF_URL, "formats": ["markdown"], "parsers": ["pdf"], "maxAge": 0}, tier=2),
    Case("scrape_pdf_parsers_object", "POST", "/v2/scrape",
         {"url": PDF_URL, "formats": ["markdown"],
          "parsers": [{"type": "pdf", "maxPages": 5}], "maxAge": 0}, tier=2),
    Case("scrape_pdf_parsers_disabled", "POST", "/v2/scrape",
         {"url": PDF_URL, "formats": ["markdown"], "parsers": [], "maxAge": 0}, tier=2),
    Case("scrape_pdf_default_autoparse", "POST", "/v2/scrape",
         {"url": PDF_URL, "formats": ["markdown"], "maxAge": 0}, tier=2),
]

# ── /v2/parse (document upload). Multipart: file + options JSON. ──
PARSE_CASES: list[Case] = [
    Case("parse_pdf_markdown", "POST", "/v2/parse",
         {"formats": ["markdown"]},
         upload_file="crates/crw-extract/tests/fixtures/sample.pdf", tier=2),
]

MAP_CASES: list[Case] = [
    Case("map_basic", "POST", "/v2/map", {"url": RICH, "limit": 10}),
]

SEARCH_CASES: list[Case] = [
    Case("search_basic", "POST", "/v2/search", {"query": "firecrawl api", "limit": 3}),
]

CRAWL_CASES: list[Case] = [
    Case("crawl_small", "POST", "/v2/crawl", {"url": EXAMPLE, "limit": 3},
         kind="job", status_path_tmpl="/v2/crawl/{id}"),
]

BATCH_CASES: list[Case] = [
    Case("batch_two_urls", "POST", "/v2/batch/scrape",
         {"urls": [EXAMPLE, RICH], "formats": ["markdown"]},
         kind="job", status_path_tmpl="/v2/batch/scrape/{id}"),
]

EXTRACT_CASES: list[Case] = [
    Case("extract_title", "POST", "/v2/extract",
         {"urls": [EXAMPLE],
          "prompt": "Extract the page title",
          "schema": {"type": "object",
                     "properties": {"title": {"type": "string"}}}},
         kind="job", status_path_tmpl="/v2/extract/{id}", tier=2),
]

ALL_CASES: list[Case] = (
    SCRAPE_CASES
    + PARSE_CASES
    + MAP_CASES
    + SEARCH_CASES
    + CRAWL_CASES
    + BATCH_CASES
    + EXTRACT_CASES
)
