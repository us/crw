"""Capture golden fixtures from the REAL Firecrawl v2 API.

    FIRECRAWL_API_KEY=fc-... uv run python -m conformance.capture

Writes one normalized `{status, body}` JSON per case under fixtures/firecrawl_v2/.
The key is read from the env and never written to disk (see .gitignore).
"""

from __future__ import annotations

import json
import os
import pathlib
import re
import sys
import time

from . import corpus
from ._http import run_case

FIRECRAWL_BASE = os.environ.get("FIRECRAWL_BASE", "https://api.firecrawl.dev")
KEY = os.environ.get("FIRECRAWL_API_KEY")
FIXDIR = pathlib.Path(__file__).resolve().parent.parent / "fixtures" / "firecrawl_v2"


def main() -> None:
    if not KEY:
        raise SystemExit("set FIRECRAWL_API_KEY (it is .gitignore'd, never committed)")
    FIXDIR.mkdir(parents=True, exist_ok=True)
    # `capture mock` drives the error/edge corpus at mock.fastcrw.com through the
    # real Firecrawl API. The mock is a public host for exactly this reason: both
    # engines can be pointed at the same deterministic fixture and diffed. The
    # captures land beside the happy-path goldens and are what `mock_parity`
    # asserts against once they exist.
    which = corpus.MOCK_CASES if "mock" in sys.argv[1:] else corpus.ALL_CASES
    for case in which:
        status, body = run_case(FIRECRAWL_BASE, KEY, case)
        # Firecrawl rate-limits per minute (17 req/min on the key this was first
        # run with) and answers 429 with the reset time in the error string. A
        # capture run is long and unattended; without this the tail of the corpus
        # silently fills up with 429 bodies that look like captured fixtures.
        for _ in range(6):
            if status != 429:
                break
            wait = 20
            m = re.search(r"retry after (\d+)s", str(body.get("error", "")))
            if m:
                wait = int(m.group(1)) + 3
            print(f"  rate limited on {case.name}; waiting {wait}s")
            time.sleep(wait)
            status, body = run_case(FIRECRAWL_BASE, KEY, case)
        if status == 429:
            print(f"  SKIP {case.name}: still rate limited, fixture NOT written")
            continue
        out = FIXDIR / f"{case.name}.json"
        out.write_text(json.dumps({"status": status, "body": body}, indent=2))
        print(f"captured {case.name}: HTTP {status} -> fixtures/firecrawl_v2/{out.name}")


if __name__ == "__main__":
    main()
