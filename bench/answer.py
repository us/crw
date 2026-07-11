#!/usr/bin/env python3
"""Answer track — end-to-end answer quality on FRAMES (the metric FRAMES is
actually designed for, unlike the Wikipedia-URL relevance proxy).

For each FRAMES question:
  1. POST /v1/search {answer:true, scrapeOptions.formats:[markdown]} → synthesized answer
  2. an independent judge LLM grades the answer PASS/FAIL against the gold Answer
  3. emit an `answer`-track `pass` row (1/0) to the shared items.jsonl

Single-system pass rate (absolute crw-vs-gold). The judge is pinned temp=0 and
recorded in the manifest by its provider/model. Answer content that comes back
empty is a genuine FAIL (value 0, status ok) — only a search infra timeout/error
is excluded (status timeout/error), matching the missing-pair policy.

Config via env (secrets never written to disk):
  CRW_API_URL, CRW_API_KEY            — fastCRW endpoint under test
  JUDGE_BASE_URL, JUDGE_API_KEY, JUDGE_MODEL — OpenAI-compatible judge
"""

from __future__ import annotations

import argparse
import asyncio
import os

import aiohttp

from schema import append_items, item_record, read_manifest

CRW_URL = os.getenv("CRW_API_URL", "http://localhost:3000")
CRW_API_KEY = os.getenv("CRW_API_KEY", "")
JUDGE_BASE_URL = os.getenv("JUDGE_BASE_URL", "")
JUDGE_API_KEY = os.getenv("JUDGE_API_KEY", "")
JUDGE_MODEL = os.getenv("JUDGE_MODEL", "")
SEARCH_LIMIT = int(os.getenv("BENCH_SEARCH_LIMIT", "5"))
SEARCH_TIMEOUT = int(os.getenv("BENCH_TIMEOUT", "90"))

GRADER_SYS = (
    "You grade whether a predicted answer is correct for a question, given the "
    "gold answer. Reply with exactly one word: PASS if the prediction states the "
    "same fact as the gold answer (allow paraphrase, extra detail, or different "
    "formatting), otherwise FAIL. If the prediction says it cannot find or does "
    "not know the answer, reply FAIL."
)


def _headers(key: str) -> dict:
    h = {"Content-Type": "application/json"}
    if key:
        h["Authorization"] = f"Bearer {key}"
    return h


async def search_answer(session, query: str) -> tuple[str, str]:
    """POST /v1/search answer:true → (answer_text, status)."""
    body = {
        "query": query,
        "answer": True,
        "limit": SEARCH_LIMIT,
        "scrapeOptions": {"formats": ["markdown"]},
    }
    try:
        async with session.post(
            f"{CRW_URL}/v1/search",
            json=body,
            headers=_headers(CRW_API_KEY),
            timeout=aiohttp.ClientTimeout(total=SEARCH_TIMEOUT),
        ) as resp:
            payload = await resp.json()
    except asyncio.TimeoutError:
        return "", "timeout"
    except Exception:  # noqa: BLE001
        return "", "error"
    ans = (payload.get("answer") or "").strip()
    if not ans:
        return "", "empty_answer"  # search ran but produced no answer → a FAIL
    return ans, "ok"


async def judge(session, question: str, gold: str, pred: str) -> bool:
    """Grade pred against gold via the judge LLM (temp 0). PASS→True."""
    user = f"Question: {question}\nGold answer: {gold}\nPredicted answer: {pred}"
    body = {
        "model": JUDGE_MODEL,
        "messages": [
            {"role": "system", "content": GRADER_SYS},
            {"role": "user", "content": user},
        ],
        "temperature": 0,
        "max_tokens": 8,
    }
    async with session.post(
        JUDGE_BASE_URL,
        json=body,
        headers=_headers(JUDGE_API_KEY),
        timeout=aiohttp.ClientTimeout(total=60),
    ) as resp:
        data = await resp.json()
    verdict = (data["choices"][0]["message"]["content"] or "").strip().upper()
    return verdict.startswith("PASS")


async def one(session, run_id, idx, row, sem) -> dict:
    q, gold = row["Prompt"], row["Answer"]
    async with sem:
        ans, status = await search_answer(session, q)
        if status in ("timeout", "error"):
            return item_record(run_id, "answer", f"q{idx}", "crw", "pass", 0.0, status)
        if status == "empty_answer":
            return item_record(run_id, "answer", f"q{idx}", "crw", "pass", 0.0, "ok")
        try:
            passed = await judge(session, q, gold, ans)
        except Exception:  # noqa: BLE001 — judge failure ≠ crw fail; exclude it
            return item_record(run_id, "answer", f"q{idx}", "crw", "pass", 0.0, "error")
        return item_record(run_id, "answer", f"q{idx}", "crw", "pass",
                           1.0 if passed else 0.0, "ok")


async def amain(run_id: str, limit: int, concurrency: int) -> int:
    read_manifest(run_id)  # fail early if the run wasn't init'd
    for name, val in (("JUDGE_BASE_URL", JUDGE_BASE_URL), ("JUDGE_API_KEY", JUDGE_API_KEY),
                      ("JUDGE_MODEL", JUDGE_MODEL)):
        if not val:
            raise SystemExit(f"missing env {name}")
    from datasets import load_dataset
    ds = load_dataset("google/frames-benchmark", split="test")
    rows = [{"Prompt": r["Prompt"], "Answer": r["Answer"]} for r in ds][:limit]
    sem = asyncio.Semaphore(concurrency)
    passed = done = 0
    async with aiohttp.ClientSession() as session:
        tasks = [one(session, run_id, i, r, sem) for i, r in enumerate(rows)]
        for coro in asyncio.as_completed(tasks):
            rec = await coro
            append_items(run_id, [rec])  # crash-safe: persist each item as it lands
            done += 1
            if rec["status"] == "ok" and rec["value"] == 1.0:
                passed += 1
            if done % 10 == 0 or done == len(rows):
                print(f"  [{done}/{len(rows)}] running pass≈{passed}/{done}")
    print(f"answer: {passed}/{done} PASS on the judge (status=ok rows)")
    return 0


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("run_id")
    p.add_argument("--limit", type=int, default=100)
    p.add_argument("--concurrency", type=int, default=5)
    a = p.parse_args()
    return asyncio.run(amain(a.run_id, a.limit, a.concurrency))


if __name__ == "__main__":
    raise SystemExit(main())
