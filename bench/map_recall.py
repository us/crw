#!/usr/bin/env python3
"""CRW /map recall benchmark.

Measures how completely `/map` discovers a site's URLs, against ground truth we
control. Primary truth source: purpose-built scraper sandboxes whose structure
is fixed and publicly known (books.toscrape.com = 1000 products, 50 listing
pages, 50 categories). Deterministic, unprotected, free — the closest thing to a
turnkey map test set.

Metric that matters is recall: `/map` trades completeness for speed (Firecrawl's
own /map finds ~3x fewer links than /crawl), so "how many URLs" alone is
misleading — see trendyol.com, which returns ~5k URLs that are almost all
foreign-locale category pages with ~0 of the actual product catalog.

Run against a local crw server:
    CRW_API_URL=http://localhost:3000 uv run python bench/map_recall.py

Offline scorer self-check (no server, uses a recorded fixture):
    uv run python bench/map_recall.py --selfcheck
"""

import json
import os
import re
import sys
import time
import urllib.request
from dataclasses import dataclass, field

CRW_URL = os.getenv("CRW_API_URL", "http://localhost:3000")
TIMEOUT = int(os.getenv("BENCH_TIMEOUT", "180"))
RESULTS_PATH = os.getenv("BENCH_RESULTS_PATH", "bench/map_results.json")
FIXTURE = "bench/fixtures/books_toscrape_map.json"


@dataclass
class Bucket:
    """A class of URLs we expect on a site, with its known ground-truth count."""

    label: str
    pattern: str  # regex matched against the full URL
    expected: int


@dataclass
class Site:
    url: str
    host: str
    buckets: list[Bucket] = field(default_factory=list)


# Ground truth = the fixed, documented structure of these sandboxes.
SITES: list[Site] = [
    Site(
        url="https://books.toscrape.com/",
        host="books.toscrape.com",
        buckets=[
            # product page: /catalogue/<slug>_<id>/index.html (not under /category/)
            Bucket("products", r"/catalogue/(?!category/)[^/]+/index\.html$", 1000),
            # main listing pagination: /catalogue/page-N.html
            Bucket("listings", r"/catalogue/page-\d+\.html$", 50),
            Bucket("categories", r"/catalogue/category/books/", 50),
        ],
    ),
    Site(
        url="https://quotes.toscrape.com/",
        host="quotes.toscrape.com",
        # 10 pages of quotes; the root counts as page 1.
        buckets=[Bucket("pages", r"/page/\d+/?$", 9)],
    ),
]


def score(links: list[str], site: Site) -> dict:
    """Bucket discovered links and compute recall/precision/junk for one site."""
    in_domain = [u for u in links if _host(u) == site.host]
    off_domain = len(links) - len(in_domain)

    buckets = {}
    for b in site.buckets:
        rx = re.compile(b.pattern)
        found = sum(1 for u in in_domain if rx.search(u))
        buckets[b.label] = {
            "found": found,
            "expected": b.expected,
            "recall": round(min(found / b.expected, 1.0), 3) if b.expected else None,
        }
    return {
        "url": site.url,
        "total": len(links),
        "in_domain": len(in_domain),
        "off_domain": off_domain,
        "junk_pct": round(100 * off_domain / len(links), 1) if links else 0.0,
        "buckets": buckets,
    }


def _host(u: str) -> str:
    m = re.match(r"https?://([^/]+)", u)
    return m.group(1).lower() if m else ""


def run_live(site: Site) -> tuple[list[str], float]:
    """POST /v1/map and return (links, elapsed_seconds)."""
    body = json.dumps({"url": site.url}).encode()
    req = urllib.request.Request(
        f"{CRW_URL}/v1/map",
        data=body,
        headers={"Content-Type": "application/json"},
    )
    t0 = time.monotonic()
    with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
        payload = json.load(resp)
    elapsed = time.monotonic() - t0
    links = payload.get("links") or payload.get("data", {}).get("links", [])
    return links, elapsed


def print_table(rows: list[dict]) -> None:
    for r in rows:
        print(f"\n{r['url']}")
        print(
            f"  total={r['total']}  in_domain={r['in_domain']}  "
            f"junk={r['junk_pct']}%  time={r.get('elapsed_s', '-')}s"
        )
        for label, b in r["buckets"].items():
            bar = "OK " if (b["recall"] or 0) >= 0.95 else "LOW"
            print(
                f"    [{bar}] {label:<12} {b['found']:>5}/{b['expected']:<5} "
                f"recall={b['recall']}"
            )


def selfcheck() -> int:
    """Score the recorded books.toscrape fixture — no server needed.

    Pins the observed baseline (~0.71 product recall at default maxDepth=2) so a
    scorer regression fails loudly. The gap itself is the finding: a clean,
    unprotected, sitemap-less static site still loses ~29% of products because
    map's BFS is depth-limited and does not follow pagination chains past depth.
    """
    with open(FIXTURE) as f:
        links = json.load(f)["links"]
    books = SITES[0]
    r = score(links, books)
    pr = r["buckets"]["products"]["recall"]
    print_table([r])
    assert r["off_domain"] == 0, f"expected no off-domain leak, got {r['off_domain']}"
    assert 0.68 <= pr <= 0.74, f"product recall baseline drifted: {pr}"
    print(f"\nselfcheck OK — product recall {pr} (baseline ~0.71)")
    return 0


def main() -> int:
    if "--selfcheck" in sys.argv:
        return selfcheck()

    rows = []
    for site in SITES:
        try:
            links, elapsed = run_live(site)
            r = score(links, site)
            r["elapsed_s"] = round(elapsed, 1)
        except Exception as e:  # noqa: BLE001 — bench wants the error inline, not a crash
            r = {
                "url": site.url,
                "error": str(e),
                "total": 0,
                "in_domain": 0,
                "off_domain": 0,
                "junk_pct": 0.0,
                "buckets": {},
            }
        rows.append(r)

    print_table(rows)
    with open(RESULTS_PATH, "w") as f:
        json.dump(rows, f, indent=2)
    print(f"\nwrote {RESULTS_PATH}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
