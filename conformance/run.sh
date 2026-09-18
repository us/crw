#!/usr/bin/env bash
# Firecrawl v2 conformance runner. Honors repo tooling (uv).
#
#   ./run.sh capture      # capture golden fixtures from the real api.firecrawl.dev
#   ./run.sh capture mock # same, for the error/edge corpus at mock.fastcrw.com
#   ./run.sh compare      # diff crw's responses against the golden fixtures
#   ./run.sh parity       # diff crw's error-path BEHAVIOUR against Firecrawl's
#   ./run.sh sdk          # run the real firecrawl-py SDK against crw (issue #62)
#   ./run.sh all          # compare + parity + sdk (the CI gate)
#
# Env: FIRECRAWL_API_KEY (capture), CRW_URL / CRW_API_KEY (compare, parity, sdk),
#      MOCK_URL (default https://mock.fastcrw.com; set to http://127.0.0.1:9372
#      to run offline against a local mock-server, which also needs the engine
#      started with CRW_ALLOW_LOOPBACK_FOR_TESTS=1).
set -euo pipefail
cd "$(dirname "$0")"

case "${1:-all}" in
  capture) uv run python -m conformance.capture "${2:-}" ;;
  compare) uv run python -m conformance.compare ;;
  parity)  uv run python -m conformance.mock_parity ;;
  sdk)     uv run python test_sdk.py ;;
  all)     uv run python -m conformance.compare \
             && uv run python -m conformance.mock_parity \
             && uv run python test_sdk.py ;;
  *) echo "usage: run.sh [capture [mock]|compare|parity|sdk|all]"; exit 1 ;;
esac
