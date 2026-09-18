"""Error/edge parity against Firecrawl v2, driven through the mock fixture server.

    CRW_URL=http://localhost:3000 uv run python -m conformance.mock_parity

`compare.py` diffs response SHAPE against captured goldens. This diffs
BEHAVIOUR: for a given target response, does our `/v2/scrape` make the same
call Firecrawl makes about success, status and final URL?

Every expectation below cites the Firecrawl source it is read out of, because
this environment has no FIRECRAWL_API_KEY and cannot capture the real thing.
Once a key exists, `./run.sh capture` can drive corpus.MOCK_CASES through the
live API — mock.fastcrw.com is public precisely so that both sides can reach
the same fixtures — and these hand-derived expectations get replaced by
captures.

The governing rule (scrapeURL/index.ts, "Success factors"):

    const isGoodStatusCode = (s >= 200 && s < 300) || s === 304;
    const hasRequiredOutput = isParsedImage || isLongEnough || !isGoodStatusCode;

`!isGoodStatusCode` is an OR term, so a bad status is SUFFICIENT to accept the
result. controllers/v2/scrape.ts then never reads metadata.statusCode — it
returns `{success: true, data: doc}`. In Firecrawl, `success:false` means our
own infrastructure failed, never "the target said 404".
"""

from __future__ import annotations

import os
import sys

from . import corpus
from ._http import run_case

CRW_URL = os.environ.get("CRW_URL", "http://localhost:3000")
KEY = os.environ.get("CRW_API_KEY", "")


class Expect:
    """What Firecrawl v2 would answer for this case.

    http / success / status_code / url_suffix are asserted when set.
    `code` is checked against the envelope's `code` (Firecrawl's key).
    `report_only` records the answer without failing the run — used where
    Firecrawl's behaviour depends on which engine in its waterfall won, which
    source alone cannot settle.
    """

    def __init__(self, why, http=None, success=None, status_code=None,
                 url_suffix=None, redirected=False, code=None, report_only=False):
        self.why = why
        self.http = http
        self.success = success
        self.status_code = status_code
        self.url_suffix = url_suffix
        # Assert metadata.url != metadata.sourceURL. Load-bearing on its own:
        # `/redirect-to?url=/json` ends in "/json" BEFORE it redirects, so a
        # suffix check alone passes against the un-redirected request url.
        self.redirected = redirected
        self.code = code
        self.report_only = report_only


_STATUS = ("captured: HTTP 200, success:true, metadata.statusCode=<code>, "
           "metadata.error=<reason phrase>. scrapeURL/index.ts accepts a bad "
           "status via `!isGoodStatusCode` and the controller never reads it")
_REDIR = ("captured: metadata.sourceURL = requested url, metadata.url = the url "
          "after redirects (scrapeURL/index.ts:1174-1175)")
_THIN = ("captured: success:true with the sentinel present. We 500 - chrome "
         "renders it, then structural_failure discards it as minimal_text. "
         "Firecrawl's bar is any non-empty text.")
_ENGINES = ("captured: HTTP 500 SCRAPE_ALL_ENGINES_FAILED. Nothing extractable, "
            "so every engine in the waterfall reports unsuccessful and "
            "NoEnginesLeftError falls to the controller's catch-all 500")

# Some rows below are `report_only`: Firecrawl does WORSE than we do on content
# we handle correctly. Recorded, never matched — matching their API contract is
# not the same as failing to parse a page we can already parse.

EXPECTATIONS: dict[str, Expect] = {
    # ── Status codes ────────────────────────────────────────────────────
    "mock_status_200": Expect("control", http=200, success=True, status_code=200),
    "mock_status_401": Expect(_STATUS, http=200, success=True, status_code=401),
    "mock_status_403": Expect(_STATUS, http=200, success=True, status_code=403),
    "mock_status_404": Expect(_STATUS, http=200, success=True, status_code=404),
    "mock_status_429": Expect(_STATUS, http=200, success=True, status_code=429),
    "mock_status_500": Expect(_STATUS, http=200, success=True, status_code=500),
    "mock_status_503": Expect(_STATUS, http=200, success=True, status_code=503),

    # ── Redirects: the case the golden corpus structurally cannot see ──
    # Every pre-existing fixture targets a non-redirecting page, so
    # sourceURL == url holds in all 11 of them by accident.
    "mock_redirect_chain": Expect(_REDIR, http=200, success=True, status_code=200,
                                  redirected=True, url_suffix="/html/article"),
    "mock_redirect_to": Expect(_REDIR, http=200, success=True, status_code=200,
                               redirected=True, url_suffix="/json"),
    "mock_redirect_relative": Expect(_REDIR, http=200, success=True, status_code=200,
                                     redirected=True, url_suffix="/html/article"),
    # No capture: /redirect/preserve-307|308 were not reached before the key's
    # rate limit; expectation still from source.
    "mock_redirect_307": Expect(_REDIR, http=200, success=True, status_code=200,
                                redirected=True, url_suffix="/echo"),
    "mock_redirect_308": Expect(_REDIR, http=200, success=True, status_code=200,
                                redirected=True, url_suffix="/echo"),
    "mock_redirect_loop": Expect(
        "the live API never answered - it held the connection past a 120s read "
        "timeout instead of returning an error. We answer 500 "
        "SCRAPE_ALL_ENGINES_FAILED promptly, which is strictly better than "
        "hanging, so there is nothing here to match.",
        report_only=True),

    # ── Walls: a status code plus a body that looks like a block ────────
    "mock_wall_captcha": Expect(
        "capture unusable: the live API answered 408 CONCURRENCY_QUEUE_TIMEOUT, "
        "which is its own capacity artifact, not a verdict on the page. The "
        "sibling 401/429 captures both confirm the status-code rule, so this "
        "row is recorded rather than asserted until a clean capture exists.",
        report_only=True),
    "mock_wall_login": Expect(_STATUS, http=200, success=True, status_code=401),
    "mock_rate_limit": Expect(_STATUS, http=200, success=True, status_code=429),

    # ── Content types ───────────────────────────────────────────────────
    "mock_json": Expect("captured: parsed, 200", http=200, success=True, status_code=200),
    "mock_xml": Expect("captured: parsed, 200", http=200, success=True, status_code=200),
    "mock_csv": Expect(
        "captured: HTTP 500 SCRAPE_RETRY_LIMIT (document_antibot). The live API "
        "routes text/csv down its document path and gives up on a 3-line CSV. "
        "We parse it - recorded, not matched.",
        report_only=True),
    "mock_text": Expect("captured: parsed, 200", http=200, success=True, status_code=200),
    "mock_binary": Expect(
        _ENGINES + ". Note this is NOT SCRAPE_UNSUPPORTED_FILE_ERROR - that code "
        "is the file-UPLOAD path only; a URL serving binary just exhausts the "
        "engine waterfall.",
        http=500, success=False, code="SCRAPE_ALL_ENGINES_FAILED"),
    "mock_content_type_lie": Expect(
        "captured: HTTP 200 success:true statusCode 200 - the live API sniffs "
        "the body rather than trusting a lying application/octet-stream header, "
        "same as we do.",
        http=200, success=True, status_code=200),

    # ── JS / CSR ────────────────────────────────────────────────────────
    # A parity gap we are NOT closing here, recorded so it is not lost.
    # Captured live: all three come back success:true with the fixture's
    # sentinel in the markdown (43, 23 and 17 chars respectively). We answer 500
    # SCRAPE_ALL_ENGINES_FAILED — chrome renders them fine and our
    # `structural_failure` classifier then discards the render as
    # "minimal_text on small page".
    #
    # Firecrawl's bar is `checkMarkdown.trim().length > 0`. Ours is a heuristic
    # that exists to catch JS shells and challenge pages, and loosening it is
    # exactly how a Cloudflare interstitial gets billed as content (see
    # `v2_verdict`). So this is a deliberate trade, not an oversight — but it
    # does mean a legitimately short page fails here and succeeds there.
    "mock_js_csr": Expect(_THIN, report_only=True),
    "mock_js_hydrate": Expect(_THIN, report_only=True),
    "mock_js_fetch": Expect(_THIN, report_only=True),

    # ── HTML shapes ─────────────────────────────────────────────────────
    "mock_html_article": Expect("control", http=200, success=True, status_code=200),
    "mock_html_empty": Expect(
        _ENGINES + ". Precedence check: `isLongEnough || !isGoodStatusCode` "
        "means an empty body only fails when the status was GOOD, so 200+empty "
        "fails and 404+empty succeeds. NOTE: with browser tiers configured we "
        "answer 408 SCRAPE_TIMEOUT instead - an empty page escalates through "
        "chrome AND lightpanda and burns the deadline before anything can "
        "conclude. Same end state, reached expensively; see FIRECRAWL-DIFF.md.",
        http=500, success=False, code="SCRAPE_ALL_ENGINES_FAILED"),
    "mock_html_malformed": Expect(
        "captured: HTTP 500 SCRAPE_ALL_ENGINES_FAILED. The fixture is valid "
        "prose behind unclosed tags and every live engine gave up on it; our "
        "parser recovers the text - recorded, not matched.",
        report_only=True),
}


def _probe(body):
    """Pull the fields we assert on out of whatever envelope came back."""
    if not isinstance(body, dict):
        return {}
    data = body.get("data") or {}
    meta = data.get("metadata") or {} if isinstance(data, dict) else {}
    return {
        "success": body.get("success"),
        "status_code": meta.get("statusCode"),
        "url": meta.get("url"),
        "source_url": meta.get("sourceURL"),
        "code": body.get("code"),
        "error_code": body.get("errorCode"),
        "error": body.get("error"),
    }


def main() -> None:
    fails: list[str] = []
    print(f"crw:  {CRW_URL}")
    print(f"mock: {corpus.MOCK}\n")

    for case in corpus.MOCK_CASES:
        exp = EXPECTATIONS.get(case.name)
        if exp is None:
            continue
        try:
            http, body = run_case(CRW_URL, KEY, case)
        except Exception as e:  # noqa: BLE001 - a transport failure is a result
            print(f"[ERROR] {case.name}: {type(e).__name__}: {e}")
            fails.append(case.name)
            continue

        got = _probe(body)
        diffs = []
        if exp.http is not None and http != exp.http:
            diffs.append(f"http {http} != {exp.http}")
        if exp.success is not None and got.get("success") is not exp.success:
            diffs.append(f"success {got.get('success')} != {exp.success}")
        if exp.status_code is not None and got.get("status_code") != exp.status_code:
            diffs.append(f"metadata.statusCode {got.get('status_code')} != {exp.status_code}")
        if exp.url_suffix is not None:
            url = got.get("url") or ""
            if not url.endswith(exp.url_suffix):
                diffs.append(f"metadata.url {url!r} does not end with {exp.url_suffix!r}")
        if exp.redirected and got.get("url") == got.get("source_url"):
            diffs.append(
                f"metadata.url == metadata.sourceURL ({got.get('url')!r}); "
                "the final URL after redirects is not being reported")
        if exp.code is not None and got.get("code") != exp.code:
            diffs.append(f"code {got.get('code')!r} != {exp.code!r} (errorCode={got.get('error_code')!r})")

        if exp.report_only:
            print(f"[note] {case.name}: http={http} success={got.get('success')} "
                  f"code={got.get('code') or got.get('error_code')}")
            print(f"       {exp.why}")
        elif diffs:
            print(f"[FAIL] {case.name}")
            for d in diffs:
                print(f"       {d}")
            print(f"       firecrawl: {exp.why}")
            if got.get("error"):
                print(f"       ours said: {str(got['error'])[:160]}")
            fails.append(case.name)
        else:
            print(f"[ok]   {case.name}")

    total = sum(1 for c in corpus.MOCK_CASES
                if c.name in EXPECTATIONS and not EXPECTATIONS[c.name].report_only)
    print(f"\n=== mock parity: {total - len(fails)}/{total} match Firecrawl v2 ===")
    if fails:
        print("diverging: " + ", ".join(fails))
        sys.exit(1)


if __name__ == "__main__":
    main()
