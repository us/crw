#!/usr/bin/env python3
"""Three-way recall comparison: crw vs crawl4ai vs firecrawl on the same 150
diagnose URLs.

Fair-matching rule applied uniformly to each tool's output:
    haystack = lower(md + "\n" + strip_md_links(md))
    truth_recall = hits / total >= 0.3 -> truth_found
where strip_md_links replaces `[text](url)` with `text`. This removes
markdown's link-syntax interference without favoring crw's plainText format.
"""

import argparse
import asyncio
import json
import os
import re
import time

import aiohttp
from datasets import load_dataset

LINK_RE = re.compile(r"\[([^\]]*)\]\([^)]*\)")

# Standardize on CRW_API_URL (default 3000, matching map_recall/run_bench and
# docker-compose's 3000:3000). The old hardcoded :3030 had no override.
CRW_URL = os.getenv("CRW_API_URL", "http://localhost:3000")


def split_phrases(text: str, min_len: int) -> list[str]:
    return [w.strip() for w in text.split("\n") if len(w.strip()) > min_len]


def match_phrases(hay: str, phrases: list[str]) -> tuple[list[str], list[str]]:
    hits, misses = [], []
    for p in phrases:
        (hits if p.lower() in hay else misses).append(p)
    return hits, misses


def build_haystack(md: str) -> str:
    if not md:
        return ""
    return (md + "\n" + LINK_RE.sub(r"\1", md)).lower()


async def call_crw(session, url: str, timeout: int) -> dict:
    body = {"url": url, "formats": ["markdown"]}
    t0 = time.monotonic()
    async with session.post(
        f"{CRW_URL}/v1/scrape",
        json=body,
        timeout=aiohttp.ClientTimeout(total=timeout),
    ) as resp:
        latency = (time.monotonic() - t0) * 1000
        body = await resp.json()
        md = ((body.get("data") or {}).get("markdown")) or ""
        return {"md": md, "latency_ms": latency, "status": resp.status,
                "ok": bool(body.get("success")) and bool(md)}


async def call_crawl4ai(session, url: str, timeout: int) -> dict:
    body = {"url": url, "f": "fit"}
    t0 = time.monotonic()
    async with session.post(
        "http://localhost:11235/md",
        json=body,
        timeout=aiohttp.ClientTimeout(total=timeout),
    ) as resp:
        latency = (time.monotonic() - t0) * 1000
        data = await resp.json()
        md = data.get("markdown") or ""
        return {"md": md, "latency_ms": latency, "status": resp.status,
                "ok": bool(data.get("success")) and bool(md)}


async def call_firecrawl(session, url: str, timeout: int) -> dict:
    body = {"url": url, "formats": ["markdown"]}
    headers = {"Authorization": "Bearer fc-local-bench"}
    t0 = time.monotonic()
    async with session.post(
        "http://localhost:3022/v1/scrape",
        json=body,
        headers=headers,
        timeout=aiohttp.ClientTimeout(total=timeout),
    ) as resp:
        latency = (time.monotonic() - t0) * 1000
        data = await resp.json()
        md = ((data.get("data") or {}).get("markdown")) or ""
        return {"md": md, "latency_ms": latency, "status": resp.status,
                "ok": bool(data.get("success")) and bool(md)}


CALLERS = {"crw": call_crw, "crawl4ai": call_crawl4ai, "firecrawl": call_firecrawl}


async def scrape_one(session, tool: str, row: dict, sem, timeout: int) -> dict:
    url = row["url"]
    truth = row.get("truth_text") or ""
    rec = {"url": url, "tool": tool, "ok": False, "status": 0,
           "latency_ms": 0, "error": None, "md_len": 0,
           "truth_total": 0, "truth_hit": 0, "truth_recall": 0.0,
           "truth_found": False}
    async with sem:
        try:
            r = await CALLERS[tool](session, url, timeout)
            rec.update({"ok": r["ok"], "status": r["status"],
                        "latency_ms": r["latency_ms"], "md_len": len(r["md"])})
            if r["md"]:
                hay = build_haystack(r["md"])
                phrases = split_phrases(truth, 20)
                if phrases:
                    hits, _ = match_phrases(hay, phrases)
                    rec["truth_total"] = len(phrases)
                    rec["truth_hit"] = len(hits)
                    rec["truth_recall"] = round(len(hits) / len(phrases), 3)
                    rec["truth_found"] = rec["truth_recall"] >= 0.3
        except asyncio.TimeoutError:
            rec["error"] = "timeout"
            rec["latency_ms"] = timeout * 1000
        except Exception as e:
            rec["error"] = str(e)[:200]
    return rec


async def amain():
    p = argparse.ArgumentParser()
    p.add_argument("--max-urls", type=int, default=150)
    p.add_argument("--timeout", type=int, default=60)
    p.add_argument("--concurrency", type=int, default=4)
    p.add_argument("--tools", default="crw,crawl4ai,firecrawl")
    p.add_argument("--out", default="bench/server-runs/diagnose_3way.jsonl")
    args = p.parse_args()

    print("Loading dataset…")
    ds = load_dataset("firecrawl/scrape-content-dataset-v1", split="train")
    rows = list(ds)[: args.max_urls]
    tools = [t.strip() for t in args.tools.split(",") if t.strip()]
    print(f"{len(rows)} URLs x {len(tools)} tools = {len(rows)*len(tools)} requests")

    sem = asyncio.Semaphore(args.concurrency)
    os.makedirs(os.path.dirname(args.out), exist_ok=True)

    results = []
    with open(args.out, "w") as f:
        async with aiohttp.ClientSession() as session:
            tasks = [scrape_one(session, t, r, sem, args.timeout)
                     for t in tools for r in rows]
            done = 0
            for coro in asyncio.as_completed(tasks):
                rec = await coro
                results.append(rec)
                f.write(json.dumps(rec, ensure_ascii=False) + "\n")
                done += 1
                if done % 50 == 0 or done == len(tasks):
                    print(f"  [{done}/{len(tasks)}]")

    print(f"\nSaved {args.out}\n")
    print("=" * 60)
    print(f"{'tool':<12} {'found':>6} {'%':>7} {'avg_recall':>12} {'avg_ms':>9} {'errors':>7}")
    print("-" * 60)
    for t in tools:
        sub = [r for r in results if r["tool"] == t]
        ok = sum(1 for r in sub if r["ok"])
        found = sum(1 for r in sub if r["truth_found"])
        avg_r = sum(r["truth_recall"] for r in sub) / len(sub) if sub else 0
        avg_ms = sum(r["latency_ms"] for r in sub if r["ok"]) / max(ok, 1)
        errs = sum(1 for r in sub if r["error"])
        print(f"{t:<12} {found:>6} {found/len(sub)*100:>6.2f}% {avg_r:>12.3f} {avg_ms:>9.0f} {errs:>7}")


if __name__ == "__main__":
    asyncio.run(amain())
