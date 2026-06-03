"""The literal issue-#62 acceptance test.

The official firecrawl-py SDK, pointed at a self-hosted crw, must run its core
methods without a 404 and return SDK-parseable objects.

    CRW_URL=http://localhost:3000 CRW_API_KEY=local uv run python test_sdk.py

Method names follow firecrawl-py v4.x. If a pinned SDK version renamed one,
adjust the lambdas below — the point is the wire contract, not the method name.
"""

from __future__ import annotations

import os

from firecrawl import FirecrawlApp

CRW_URL = os.environ.get("CRW_URL", "http://localhost:3000")
KEY = os.environ.get("CRW_API_KEY", "local")

SCHEMA = {"type": "object", "properties": {"title": {"type": "string"}}}


def main() -> None:
    app = FirecrawlApp(api_url=CRW_URL, api_key=KEY)
    failures: list[tuple[str, str]] = []

    def check(name: str, fn) -> None:
        try:
            r = fn()
            print(f"[ok]   {name}: {type(r).__name__}")
        except Exception as e:  # noqa: BLE001 — any SDK-side error is a failure
            failures.append((name, repr(e)))
            print(f"[FAIL] {name}: {e}")

    check("scrape", lambda: app.scrape("https://example.com", formats=["markdown"]))
    # Use a tiny, deterministic target — a large site can exceed the engine's
    # map discovery budget (120s) and time out for reasons unrelated to shape.
    check("map", lambda: app.map("https://example.com", limit=10))
    # Note: search needs a SearXNG backend (CRW_SEARCH__SEARXNG_URL) and extract
    # needs an LLM (CRW_EXTRACTION__LLM__*); without them the engine returns a
    # graceful disabled/failed status that the SDK still parses without error.
    check("search", lambda: app.search("firecrawl", limit=3))
    check("crawl", lambda: app.crawl("https://example.com", limit=3))
    check("batch_scrape", lambda: app.batch_scrape(["https://example.com"], formats=["markdown"]))
    check("extract", lambda: app.extract(urls=["https://example.com"], schema=SCHEMA))

    if failures:
        raise SystemExit(f"SDK conformance FAILED: {[f[0] for f in failures]}")
    print("\nAll 6 firecrawl-py core methods succeeded against crw.")


if __name__ == "__main__":
    main()
