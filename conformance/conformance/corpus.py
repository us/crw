"""Deterministic request corpus for the Firecrawl v2 conformance suite.

Both `capture` (vs the real api.firecrawl.dev) and `compare` (vs a local crw)
drive the SAME requests so the responses are diffable. Async endpoints
(crawl/batch/extract) are start + poll-to-terminal.
"""

from __future__ import annotations

import os
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


# ── Error / edge corpus, driven against the mock fixture server ──
#
# Every case above targets a live third-party page that returns 200. That is why
# the golden corpus cannot see a single error-path divergence, and why
# `metadata.url` being aliased to `sourceURL` went unnoticed: no fixture target
# redirects, so `sourceURL == url` holds in all of them by accident.
#
# `mock.fastcrw.com` is a public host on purpose. These cases are diffable both
# ways: `compare` drives them against a local crw, and `capture` (with a
# FIRECRAWL_API_KEY) drives the SAME urls through the real Firecrawl API, which
# can reach the mock exactly as it reaches any other site. Until someone runs
# capture, `mock_parity.py` asserts against expectations read out of Firecrawl's
# source instead.
#
# MOCK_URL points at the deployed host by default; set it to
# http://127.0.0.1:9372 to run fully offline (the engine then needs
# CRW_ALLOW_LOOPBACK_FOR_TESTS=1, which is how the local parity run works).
MOCK = os.environ.get("MOCK_URL", "https://mock.fastcrw.com").rstrip("/")


def _scrape(name: str, path: str, **body: Any) -> Case:
    return Case(name, "POST", "/v2/scrape", {"url": MOCK + path, **body})


MOCK_CASES: list[Case] = [
    # Status codes. Firecrawl returns the page with metadata.statusCode set;
    # the status never drives `success`.
    _scrape("mock_status_200", "/status/200"),
    _scrape("mock_status_401", "/status/401"),
    _scrape("mock_status_403", "/status/403"),
    _scrape("mock_status_404", "/status/404"),
    _scrape("mock_status_429", "/status/429"),
    _scrape("mock_status_500", "/status/500"),
    _scrape("mock_status_503", "/status/503"),
    # Redirects. The whole point is metadata.url != metadata.sourceURL.
    _scrape("mock_redirect_chain", "/redirect/3"),
    _scrape("mock_redirect_to", "/redirect-to?url=/json"),
    _scrape("mock_redirect_relative", "/redirect/relative"),
    _scrape("mock_redirect_307", "/redirect/preserve-307"),
    _scrape("mock_redirect_308", "/redirect/preserve-308"),
    _scrape("mock_redirect_loop", "/redirect/loop"),
    # Walls: a status code plus a body that looks like a block.
    _scrape("mock_wall_captcha", "/wall/captcha"),
    _scrape("mock_wall_login", "/wall/login"),
    _scrape("mock_rate_limit", "/rate-limit"),
    # Content types.
    _scrape("mock_json", "/json"),
    _scrape("mock_xml", "/xml"),
    _scrape("mock_csv", "/csv"),
    _scrape("mock_text", "/text"),
    # Genuinely binary: Buffer.alloc(1024) of NULs under application/octet-stream.
    # NOT /content-type?type=... — that one serves an HTML body under a lying
    # header, so it is a header-honesty case, not an unsupported-body case.
    _scrape("mock_binary", "/bytes/1024"),
    _scrape("mock_content_type_lie", "/content-type?type=application/octet-stream"),
    # JS / CSR. Both engines render these correctly; what differs is the bar
    # for "did we get content". Firecrawl accepts any non-empty text
    # (`isLongEnough` is `trim().length > 0`); our structural_failure classifier
    # rejects a small page as minimal_text. /js/hydrate renders to 23 chars.
    _scrape("mock_js_csr", "/js/csr?delay=500"),
    _scrape("mock_js_hydrate", "/js/hydrate"),
    _scrape("mock_js_fetch", "/js/fetch"),
    # HTML shapes.
    _scrape("mock_html_article", "/html/article"),
    _scrape("mock_html_empty", "/html/empty"),
    _scrape("mock_html_malformed", "/html/malformed"),
]
