#!/usr/bin/env python3
"""Firecrawl Light API test suite — data engineer perspective."""

import json
import re
import time
import requests
from concurrent.futures import ThreadPoolExecutor, as_completed

BASE = "http://localhost:3000"
PASS_COUNT = 0
FAIL_COUNT = 0


def report(name: str, passed: bool, detail: str = ""):
    global PASS_COUNT, FAIL_COUNT
    tag = "PASS" if passed else "FAIL"
    if passed:
        PASS_COUNT += 1
    else:
        FAIL_COUNT += 1
    suffix = f" — {detail}" if detail else ""
    print(f"  [{tag}] {name}{suffix}")


def scrape(payload: dict, timeout: float = 30.0) -> dict:
    r = requests.post(f"{BASE}/v1/scrape", json=payload, timeout=timeout)
    return r.json()


# ── Test 1: Batch scraping pipeline ──────────────────────────────────────────
def test_batch_scraping():
    print("\n=== Test 1: Batch scraping pipeline ===")
    urls = [
        "https://example.com",
        "https://httpbin.org/html",
        "https://news.ycombinator.com",
        "https://docs.python.org/3/",
        "https://github.blog",
    ]
    results = {}
    t0 = time.time()

    def fetch(url):
        return url, scrape({"url": url, "formats": ["markdown"]})

    with ThreadPoolExecutor(max_workers=5) as pool:
        futures = {pool.submit(fetch, u): u for u in urls}
        for f in as_completed(futures):
            url, resp = f.result()
            results[url] = resp

    elapsed = time.time() - t0
    successes = sum(1 for r in results.values() if r.get("success"))
    report("All 5 URLs succeed", successes == 5,
           f"{successes}/5 succeeded")
    report(f"Total time measured", True, f"{elapsed:.2f}s for 5 concurrent scrapes")
    for url, r in results.items():
        ok = r.get("success", False)
        md_len = len(r.get("data", {}).get("markdown", ""))
        report(f"  {url}", ok, f"markdown={md_len} chars")


# ── Test 2: Data extraction quality ──────────────────────────────────────────
def test_data_extraction_quality():
    print("\n=== Test 2: Data extraction quality (Wikipedia Python) ===")
    resp = scrape({
        "url": "https://en.wikipedia.org/wiki/Python_(programming_language)",
        "formats": ["markdown", "plainText", "links"],
    }, timeout=30)
    report("Request succeeds", resp.get("success", False))
    data = resp.get("data", {})

    md = data.get("markdown", "")
    report("Markdown > 10000 chars", len(md) > 10000, f"len={len(md)}")

    pt = data.get("plainText", "")
    has_tags = bool(re.search(r"<(div|span|p|a|table|img)\b", pt))
    report("Plain text has no HTML tags", not has_tags,
           f"len={len(pt)}, html_found={has_tags}")

    links = data.get("links", [])
    report("Links list > 50 entries", isinstance(links, list) and len(links) > 50,
           f"count={len(links) if isinstance(links, list) else 'N/A'}")


# ── Test 3: Metadata completeness ────────────────────────────────────────────
def test_metadata_completeness():
    print("\n=== Test 3: Metadata completeness (Wikipedia Linux) ===")
    resp = scrape({
        "url": "https://en.wikipedia.org/wiki/Linux",
        "formats": ["markdown"],
    })
    data = resp.get("data", {})
    meta = data.get("metadata", {})

    report("title present", bool(meta.get("title")), f"title={meta.get('title', '')!r}")
    desc = meta.get("description") or ""
    report("description present", bool(desc), f"len={len(desc)}")
    src = meta.get("sourceURL", "")
    report("sourceURL starts with https://", src.startswith("https://"), f"sourceURL={src!r}")
    sc = meta.get("statusCode")
    report("statusCode is 200", sc == 200, f"statusCode={sc}")
    lang = meta.get("language")
    report("language present", bool(lang), f"language={lang!r}")


# ── Test 4: Include/exclude tags ─────────────────────────────────────────────
def test_include_exclude_tags():
    print("\n=== Test 4: Include/exclude tags for data cleaning ===")
    url = "https://en.wikipedia.org/wiki/Python_(programming_language)"
    resp_full = scrape({"url": url, "formats": ["markdown"]})
    resp_excl = scrape({
        "url": url,
        "formats": ["markdown"],
        "excludeTags": [".reflist", ".navbox", "#toc", ".mw-editsection"],
    })
    md_full = len(resp_full.get("data", {}).get("markdown", ""))
    md_excl = len(resp_excl.get("data", {}).get("markdown", ""))
    report("Full markdown received", md_full > 0, f"len={md_full}")
    report("Excluded markdown received", md_excl > 0, f"len={md_excl}")
    report("Excluded version is smaller", md_excl < md_full,
           f"full={md_full} excl={md_excl} delta={md_full - md_excl}")


# ── Test 5: Crawl for multi-page collection ──────────────────────────────────
def test_crawl_multi_page():
    print("\n=== Test 5: Crawl for multi-page data collection ===")
    r = requests.post(f"{BASE}/v1/crawl", json={
        "url": "https://httpbin.org",
        "maxDepth": 1,
        "maxPages": 3,
    }, timeout=15)
    body = r.json()
    report("Crawl accepted", body.get("success", False), f"id={body.get('id')}")
    crawl_id = body.get("id")
    if not crawl_id:
        report("Crawl ID returned", False, "no id")
        return

    # Poll until done
    for _ in range(30):
        time.sleep(2)
        status_r = requests.get(f"{BASE}/v1/crawl/{crawl_id}", timeout=10)
        status = status_r.json()
        if status.get("status") == "completed":
            break
    else:
        report("Crawl completes within 60s", False, f"status={status.get('status')}")
        return

    report("Crawl completed", status.get("status") == "completed")
    report("'total' field present", "total" in status, f"total={status.get('total')}")
    report("'completed' field present", "completed" in status,
           f"completed={status.get('completed')}")
    data = status.get("data", [])
    report("data is array of results", isinstance(data, list) and len(data) > 0,
           f"pages={len(data)}")


# ── Test 6: Error resilience ─────────────────────────────────────────────────
def test_error_resilience():
    print("\n=== Test 6: Error resilience ===")
    # 404 page
    resp404 = scrape({"url": "https://httpbin.org/status/404", "formats": ["markdown"]})
    sc = resp404.get("data", {}).get("metadata", {}).get("statusCode")
    report("404 page returns statusCode=404", sc == 404, f"statusCode={sc}")

    # Non-existent domain
    resp_bad = scrape({"url": "https://this-domain-does-not-exist-xyz123.com"})
    report("Non-existent domain returns error gracefully",
           resp_bad.get("success") is False or "error" in str(resp_bad).lower(),
           f"success={resp_bad.get('success')}")

    # Malformed request
    r = requests.post(f"{BASE}/v1/scrape", json={"not_a_url": 123}, timeout=10)
    try:
        body = r.json()
        got_error = r.status_code >= 400 or body.get("success") is False
        detail = f"status={r.status_code} body_keys={list(body.keys())}"
    except Exception:
        got_error = r.status_code >= 400
        detail = f"status={r.status_code} body=non-json ({len(r.text)} bytes)"
    report("Malformed request gets clear error", got_error, detail)


# ── Test 7: Rate limiting / concurrent load ──────────────────────────────────
def test_rate_limiting():
    print("\n=== Test 7: Rate limiting — 10 rapid requests ===")
    t0 = time.time()

    def hit(_):
        return scrape({"url": "https://example.com", "formats": ["markdown"]})

    with ThreadPoolExecutor(max_workers=10) as pool:
        results = list(pool.map(hit, range(10)))

    elapsed = time.time() - t0
    ok_count = sum(1 for r in results if r.get("success"))
    report("All 10 requests succeed", ok_count == 10, f"{ok_count}/10 ok")
    report("Total time measured", True, f"{elapsed:.2f}s for 10 requests")


# ── Test 8: Large page handling ──────────────────────────────────────────────
def test_large_page():
    print("\n=== Test 8: Large page handling ===")
    t0 = time.time()
    resp = scrape({
        "url": "https://en.wikipedia.org/wiki/Wikipedia:Featured_articles",
        "formats": ["markdown"],
    }, timeout=60)
    elapsed = time.time() - t0
    md = resp.get("data", {}).get("markdown", "")
    report("Request succeeds", resp.get("success", False))
    report("Markdown > 100KB", len(md) > 100_000,
           f"len={len(md)} ({len(md)/1024:.1f}KB)")
    report("No timeout", elapsed < 60, f"took {elapsed:.2f}s")


# ── Main ─────────────────────────────────────────────────────────────────────
if __name__ == "__main__":
    print("=" * 60)
    print("Firecrawl Light — Data Pipeline Test Suite")
    print("=" * 60)

    test_batch_scraping()
    test_data_extraction_quality()
    test_metadata_completeness()
    test_include_exclude_tags()
    test_crawl_multi_page()
    test_error_resilience()
    test_rate_limiting()
    test_large_page()

    print("\n" + "=" * 60)
    total = PASS_COUNT + FAIL_COUNT
    print(f"RESULTS: {PASS_COUNT}/{total} passed, {FAIL_COUNT} failed")
    print("=" * 60)
