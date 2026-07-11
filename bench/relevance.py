#!/usr/bin/env python3
"""W2 — Search-relevance track (single-system vs gold; paired only if a
competitor connector lands).

Gold = FRAMES `wiki_links` column (the Wikipedia articles the answer was
authored from). Each linked article URL is a binary-relevant document (grade=1).
We call `/v1/search` with `answer:false`, read `data.results` ordered by
`position`, canonicalize both sides, and score Recall@10 / nDCG@10 / MRR.

Construct-validity caveat (pre-registered, see PREREG.md): `wiki_links` are
Wikipedia authoring sources, not arbitrary web URLs — a correct answer sourced
off-Wikipedia scores as a miss. Treat the metric as DIRECTIONAL. The switch to a
hand-labeled fallback set is a pre-registered acceptance rule, NOT a post-hoc
decision after seeing scores.

Offline pieces (canonicalization, gold parse, IR metrics) are pure and tested by
the local verify command; the live search call is a thin wrapper.
"""

from __future__ import annotations

import ast
import json
import math
import os
import re
import urllib.parse
import urllib.request

from schema import content_hash, item_record

CRW_URL = os.getenv("CRW_API_URL", "http://localhost:3000")
CRW_API_KEY = os.getenv("CRW_API_KEY", "")  # required for managed endpoints
TIMEOUT = int(os.getenv("BENCH_TIMEOUT", "60"))
K = 10
SEARCH_LIMIT = int(os.getenv("BENCH_SEARCH_LIMIT", "20"))  # fetch ≥K to score Recall@10

# Query params that carry no semantics — dropped in canonicalization. Anything
# NOT on this denylist is kept (a semantic ?q=/?id= must survive).
TRACKING_PARAMS = {
    "utm_source", "utm_medium", "utm_campaign", "utm_term", "utm_content",
    "gclid", "fbclid", "msclkid", "mc_eid", "mc_cid", "_ga", "ref", "ref_src",
    "ref_url", "igshid", "spm", "yclid", "dclid", "wt_mc", "cmpid",
}

# Bump when the rules below change; hashed into the manifest so a comparison
# across two normalization schemes is refused by report.py.
URL_NORM_VERSION = "url-norm-v1"
_NORM_RULES_DESC = (
    "v1: lowercase+strip-www host; force https; urldecode; drop #frag; "
    "drop tracking params (keep semantic); strip trailing slash; "
    "wikipedia /wiki path case-preserved, spaces<->underscores unified"
)


def url_normalization_hash() -> str:
    return content_hash(URL_NORM_VERSION + "|" + _NORM_RULES_DESC)


def canonicalize(url: str) -> str:
    """Normalize a URL to a comparable key. Idempotent."""
    url = (url or "").strip()
    if not url:
        return ""
    if "://" not in url:
        url = "https://" + url
    p = urllib.parse.urlsplit(url)
    host = p.hostname or ""
    host = host.lower()
    if host.startswith("www."):
        host = host[4:]
    # Path: urldecode, preserve case (URLs are path-case-sensitive; Wikipedia
    # titles especially). Unify Wikipedia space/underscore so redirect variants
    # collapse (/wiki/Barack%20Obama == /wiki/Barack_Obama).
    path = urllib.parse.unquote(p.path)
    if "/wiki/" in path:
        path = path.replace(" ", "_")
    if len(path) > 1 and path.endswith("/"):
        path = path.rstrip("/")
    # Query: keep semantic params, drop tracking ones. Sort for stability.
    kept = [(k, v) for k, v in urllib.parse.parse_qsl(p.query, keep_blank_values=True)
            if k.lower() not in TRACKING_PARAMS]
    query = urllib.parse.urlencode(sorted(kept))
    canon = f"https://{host}{path}"
    if query:
        canon += "?" + query
    return canon


def _dedup(urls: list[str]) -> list[str]:
    """Dedup preserving first occurrence (keeps best rank for returned URLs)."""
    seen, out = set(), []
    for u in urls:
        if u and u not in seen:
            seen.add(u)
            out.append(u)
    return out


def parse_wiki_links(raw) -> list[str]:
    """FRAMES `wiki_links` (stringified python list, or list, or newline text) →
    canonical, deduped gold URLs. Empty/garbage → []."""
    if raw is None:
        return []
    if isinstance(raw, list):
        items = raw
    else:
        s = str(raw).strip()
        if not s:
            return []
        items = None
        if s[0] in "[(":
            try:
                parsed = ast.literal_eval(s)
                if isinstance(parsed, (list, tuple)):
                    items = list(parsed)
            except (ValueError, SyntaxError):
                items = None
        if items is None:
            # Fallback: split on newlines/whitespace, keep http(s) tokens.
            items = re.findall(r"https?://\S+", s) or s.split()
    return _dedup([canonicalize(str(u)) for u in items if str(u).strip()])


# --- IR metrics (k, binary relevance) --------------------------------------
def recall_at_k(ranked: list[str], gold: set[str], k: int = K) -> float:
    if not gold:
        raise ValueError("recall undefined for empty gold")
    hits = len(set(ranked[:k]) & gold)
    return hits / len(gold)


def ndcg_at_k(ranked: list[str], gold: set[str], k: int = K) -> float:
    if not gold:
        raise ValueError("ndcg undefined for empty gold")
    dcg = sum(1.0 / math.log2(i + 2) for i, u in enumerate(ranked[:k]) if u in gold)
    idcg = sum(1.0 / math.log2(i + 2) for i in range(min(len(gold), k)))
    return dcg / idcg if idcg else 0.0


def mrr(ranked: list[str], gold: set[str]) -> float:
    for i, u in enumerate(ranked):
        if u in gold:
            return 1.0 / (i + 1)
    return 0.0


def score_query(returned: list[str], gold_urls: list[str], k: int = K) -> dict | None:
    """Score one query. `returned` already ordered by rank. Returns None (=exclude,
    log it) when there is no gold — never a fake 0/0."""
    gold = set(_dedup([canonicalize(g) for g in gold_urls]))
    if not gold:
        return None
    ranked = _dedup([canonicalize(u) for u in returned])
    capped = len(gold) > k  # multi-hop: |G|>k caps Recall@k below 1 by construction
    return {
        "recall@k": recall_at_k(ranked, gold, k),
        "ndcg@k": ndcg_at_k(ranked, gold, k),
        "mrr": mrr(ranked, gold),
        "recall@|G|": recall_at_k(ranked, gold, max(k, len(gold))),
        "gold_capped": capped,
        "n_gold": len(gold),
    }


# --- Live path -------------------------------------------------------------
def search_ranked(query: str) -> tuple[list[str], str]:
    """POST /v1/search answer:false → (urls ordered by position, status)."""
    body = json.dumps({"query": query, "answer": False, "limit": SEARCH_LIMIT}).encode()
    headers = {"Content-Type": "application/json"}
    if CRW_API_KEY:
        headers["Authorization"] = f"Bearer {CRW_API_KEY}"
    req = urllib.request.Request(f"{CRW_URL}/v1/search", data=body, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT) as resp:
            payload = json.load(resp)
    except TimeoutError:
        return [], "timeout"
    except Exception:  # noqa: BLE001 — bench records the failure, does not crash
        return [], "error"
    # /v1/search serializes results at `data` directly (a list); older/self-host
    # shapes nested them under `data.results`. Handle both.
    data = payload.get("data")
    results = data if isinstance(data, list) else (data or {}).get("results") or []
    if not results:
        return [], "empty"
    ordered = sorted(results, key=lambda r: r.get("position", 1 << 30))
    return [r.get("url", "") for r in ordered], "ok"


def run_relevance(run_id: str, items: list[dict], limit: int | None = None) -> dict:
    """items = [{prompt, wiki_links}]. Emits per-query recall@k/ndcg@k/mrr rows.
    Returns a summary incl. excluded (empty-gold) count."""
    records, excluded = [], 0
    rows = items[:limit] if limit else items
    for idx, it in enumerate(rows):
        gold = parse_wiki_links(it.get("wiki_links"))
        if not gold:
            excluded += 1  # exclude query, log count — NOT scored 0
            continue
        ranked, status = search_ranked(it["prompt"])
        sc = score_query(ranked, gold)
        if sc is None:  # gold non-empty above, so this is unreachable — defensive
            continue
        item_id = f"q{idx}"
        for metric in ("recall@k", "ndcg@k", "mrr"):
            records.append(item_record(run_id, "relevance", item_id, "crw",
                                       metric, sc[metric], status))
    return {"records": records, "excluded_empty_gold": excluded,
            "scored": len(rows) - excluded}


def _selfcheck() -> int:
    # Canonicalization: host lowercase+www strip, force https, drop tracking,
    # keep semantic query, strip trailing slash, wiki space/underscore unify.
    assert canonicalize("http://WWW.Example.com/Path/") == "https://example.com/Path"
    assert canonicalize("https://x.com/a?utm_source=g&id=7") == "https://x.com/a?id=7"
    assert canonicalize("https://en.wikipedia.org/wiki/Barack%20Obama") == \
        canonicalize("https://en.wikipedia.org/wiki/Barack_Obama")
    assert canonicalize(canonicalize("http://a.com/p/")) == canonicalize("http://a.com/p/")

    # wiki_links parse: stringified list, dedup, canonicalize.
    g = parse_wiki_links("['https://en.wikipedia.org/wiki/A', "
                         "'https://en.wikipedia.org/wiki/A', "
                         "'http://en.wikipedia.org/wiki/B']")
    assert g == ["https://en.wikipedia.org/wiki/A", "https://en.wikipedia.org/wiki/B"], g
    assert parse_wiki_links("") == [] and parse_wiki_links(None) == []

    gold = {"https://en.wikipedia.org/wiki/A", "https://en.wikipedia.org/wiki/B"}
    # Perfect: both gold in top-2.
    ranked = ["https://en.wikipedia.org/wiki/A", "https://en.wikipedia.org/wiki/B"]
    assert recall_at_k(ranked, gold, 10) == 1.0
    assert abs(ndcg_at_k(ranked, gold, 10) - 1.0) < 1e-9
    assert mrr(ranked, gold) == 1.0
    # Miss at rank 1, hit at rank 2: MRR=0.5, recall=0.5 (one of two gold).
    r2 = ["https://x.com/junk", "https://en.wikipedia.org/wiki/A"]
    assert mrr(r2, gold) == 0.5
    assert recall_at_k(r2, gold, 10) == 0.5

    # Edge: empty gold → score_query returns None (exclude), not 0/0.
    assert score_query(["https://x"], []) is None
    # Edge: duplicate returned URLs are deduped (no double credit / rank inflation).
    dup = ["https://en.wikipedia.org/wiki/A", "https://en.wikipedia.org/wiki/A"]
    assert recall_at_k(_dedup([canonicalize(u) for u in dup]), gold, 10) == 0.5
    # Edge: non-contiguous / zero-based position handled by sort in search_ranked
    #   (rank comes from sorted order, not the raw position value) — verify sort.
    fake = [{"url": "b", "position": 5}, {"url": "a", "position": 0}]
    ordered = [r["url"] for r in sorted(fake, key=lambda r: r["position"])]
    assert ordered == ["a", "b"], ordered
    # Edge: |G|>k caps recall@k; recall@|G| can still reach 1.
    big_gold = {f"https://w/{i}" for i in range(15)}
    top10 = [f"https://w/{i}" for i in range(10)]
    sc = score_query(top10, list(big_gold), k=10)
    assert sc is not None
    assert sc["gold_capped"] and sc["recall@k"] == 10 / 15
    assert sc["recall@|G|"] == 10 / 15  # capped by what's returned, not by k here
    # Edge: IDCG guard — gold larger than k still yields finite nDCG in [0,1].
    assert 0.0 <= ndcg_at_k(top10, big_gold, 10) <= 1.0

    # Normalization hash is stable + non-empty (feeds the manifest same-harness guard).
    assert url_normalization_hash() == url_normalization_hash() and url_normalization_hash()
    print("relevance.py selfcheck OK — canon/parse/recall/ndcg/mrr/edges")
    return 0


if __name__ == "__main__":
    raise SystemExit(_selfcheck())
