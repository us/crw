#!/usr/bin/env bash
# CI guard for documentation consistency.
#
# Guard 1 — base-URL lint
#   Fails if the string "fastcrw.com/api" (the SaaS-only control-plane base
#   URL) appears in any doc or MCP source file OTHER than the explicitly
#   whitelisted files that intentionally describe the hosted control plane.
#
#   Whitelisted files (all occurrences allowed):
#     docs/docs/monitoring.md  — SaaS control-plane API reference; every
#                                appearance of fastcrw.com/api here is a
#                                legitimate /v1/monitor example or the
#                                "Base URL: https://fastcrw.com/api (hosted)"
#                                note.
#     docs/STYLE_GUIDE.md      — two-namespace explainer that names the URL
#                                once to contrast ENGINE vs SaaS namespaces.
#
#   Self-hosted engine endpoints use https://api.fastcrw.com (different
#   subdomain) and MUST NOT reference the SaaS control-plane base URL.
#
# Guard 2 — SKILL.md divergence check
#   docs/agent-onboarding/SKILL.md and mcp/crw-mcp/skills/SKILL.md must be
#   kept byte-for-byte identical. A diff here means one copy was updated
#   without syncing the other.
#
# Portable: bash + standard POSIX tools (grep, diff). Works on ubuntu-latest
# and macOS without extra dependencies.

set -euo pipefail

cd "${CHECK_REPO_ROOT:-$(dirname "$0")/..}"

FAIL=0

# ---------------------------------------------------------------------------
# Guard 1: base-URL lint
# ---------------------------------------------------------------------------

# Pattern to detect. Matches the SaaS control-plane base URL:
#   https://fastcrw.com/api  (no subdomain before fastcrw)
#
# Deliberately does NOT match:
#   https://api.fastcrw.com       — engine base URL (correct for engine docs)
#   https://docs.fastcrw.com/api-reference/...  — documentation site links
#
# The pattern requires the scheme + bare hostname so that "docs.fastcrw.com"
# and "api.fastcrw.com" never trigger it.
URL_PATTERN='https://fastcrw\.com/api'

# Files (relative to repo root) where the pattern is explicitly allowed.
# Must be an exact relative path match against grep's "file:line" output.
WHITELIST=(
  "docs/docs/monitoring.md"
  "docs/STYLE_GUIDE.md"
  "docs/docs/authentication.md"
  "docs/docs/recipe-monitoring.md"
  "docs/docs/troubleshooting.md"
  "docs/llms-full.txt"
  # Research routes are served by the SaaS control plane under /api/v2/...;
  # the engine mounts them only under /v1/search/research/*. The page documents
  # both surfaces, so the control-plane URL is legitimate here.
  "docs/docs/research-api.md"
)

# Build a grep -v filter for whitelisted paths. Each entry is anchored at the
# start of the grep output line so a malicious path like
# "evil/docs/docs/monitoring.md" cannot slip through.
exclude_args=()
for wl in "${WHITELIST[@]}"; do
  exclude_args+=(-e "^${wl}:")
done

echo "==> Guard 1: base-URL lint (pattern: ${URL_PATTERN})"

# Scan docs/ and mcp/ (text files only; skip generated HTML and binary blobs).
# grep -rn returns "path:lineno:content"; exclude whitelisted paths then check
# if any violations remain.
violations=$(
  grep -rn --include="*.md" --include="*.mdx" \
       --include="*.txt" --include="*.js" \
       --include="*.ts" --include="*.py" \
       --include="*.sh" --include="*.toml" \
       --include="*.json" --include="*.yaml" --include="*.yml" \
       -E "${URL_PATTERN}" \
       docs/ mcp/ 2>/dev/null \
    | grep -v "${exclude_args[@]}" \
    || true
)

if [ -n "$violations" ]; then
  echo "FAIL: the SaaS control-plane base URL (fastcrw.com/api) appears" >&2
  echo "      outside the whitelisted files. Self-hosted engine docs must" >&2
  echo "      use https://api.fastcrw.com instead." >&2
  echo >&2
  echo "Violations:" >&2
  echo "$violations" | sed 's/^/  /' >&2
  echo >&2
  echo "To allow a new legitimate use: add the file path to the WHITELIST" >&2
  echo "array in scripts/docs-guards.sh." >&2
  FAIL=1
else
  echo "ok: no SaaS control-plane base URL in unwhitelisted files"
fi

# ---------------------------------------------------------------------------
# Guard 2: SKILL.md divergence
# ---------------------------------------------------------------------------

SKILL_A="docs/agent-onboarding/SKILL.md"
SKILL_B="mcp/crw-mcp/skills/SKILL.md"

echo "==> Guard 2: SKILL.md divergence check"

if [ ! -f "$SKILL_A" ]; then
  echo "FAIL: ${SKILL_A} not found" >&2
  FAIL=1
elif [ ! -f "$SKILL_B" ]; then
  echo "FAIL: ${SKILL_B} not found" >&2
  FAIL=1
elif ! diff -u "$SKILL_A" "$SKILL_B" > /dev/null 2>&1; then
  echo "FAIL: SKILL.md copies have diverged — update both files together:" >&2
  echo "  ${SKILL_A}" >&2
  echo "  ${SKILL_B}" >&2
  echo >&2
  diff -u "$SKILL_A" "$SKILL_B" | sed 's/^/  /' >&2 || true
  FAIL=1
else
  echo "ok: SKILL.md copies are identical"
fi

# ---------------------------------------------------------------------------
# Guard 3: the search backend's identity must never appear on a user-facing
# surface. Config/contract identifiers are fine and must keep working, so we
# only reject the bare NAME in prose: any "searxng" that is not part of a config
# key, env var, docker service/image, CLI flag, or URL host.
#
# Allowed (contract the user must be able to type):
#   searxng_url, [search].searxng_url, CRW_SEARCH__SEARXNG_URL, CRW_SEARXNG_URL,
#   SEARXNG_SECRET_KEY, --searxng-url, the docker service/image/host `searxng`.

BACKEND_SURFACES=(
  README.md
  README.zh-CN.md
  COMPATIBILITY.md
  COMPATIBILITY-firecrawl.md
  docs/llms.txt
  docs/llms-full.txt
  crates/crw-server/openapi/openapi.json
  crates/crw-server/openapi/openapi-3.0.json
  crates/crw-server/src/routes/search.rs
)

# A hit is ALLOWED when the match is immediately part of an identifier: it is
# followed by _url / _SECRET / :port / / (image or host path), preceded by - or _
# or $ or / , or wrapped in the env-var prefix. Everything else is prose.
BACKEND_ALLOWED='searxng_url|SEARXNG_URL|SEARXNG_SECRET|--searxng-url|searxng/searxng|searxng:[0-9]|//searxng|-searxng|searxng-internal|my-searxng'

backend_hits=""
for f in "${BACKEND_SURFACES[@]}" $(find skills -name 'SKILL.md' 2>/dev/null) skills/README.md; do
  [ -f "$f" ] || continue
  # crw-self-host legitimately documents running the compose stack by name.
  case "$f" in skills/crw-self-host/SKILL.md) continue ;; esac

  case "$f" in
    *.rs)
      # In Rust, only STRING LITERALS reach a user (error bodies, printed text).
      # Type names (SearxngClient), fields (state.searxng) and internal comments
      # are code, not surface, so require a quote and skip comment lines.
      hits=$(grep -in "searxng" "$f" 2>/dev/null \
        | grep -v '^[0-9]*:[[:space:]]*//' \
        | grep '"' \
        | grep -Eiv "$BACKEND_ALLOWED" || true)
      ;;
    *)
      hits=$(grep -in "searxng" "$f" 2>/dev/null | grep -Eiv "$BACKEND_ALLOWED" || true)
      ;;
  esac

  if [ -n "$hits" ]; then
    backend_hits="${backend_hits}${f}:\n$(echo "$hits" | sed 's/^/  /')\n"
  fi
done

if [ -n "$backend_hits" ]; then
  echo "FAIL: the search backend's name leaked into a user-facing surface." >&2
  echo "      Say \"the search backend\" in prose. Config keys, env vars, CLI" >&2
  echo "      flags and docker service names are fine and must stay verbatim." >&2
  printf "%b" "$backend_hits" >&2
  FAIL=1
else
  echo "ok: no search-backend name in user-facing prose"
fi

# ---------------------------------------------------------------------------
# Guard 4: retired capability claims
# ---------------------------------------------------------------------------
#
# These features SHIP. The docs used to say otherwise, and a stale claim is a
# lie a caller can act on (they design around a feature we actually have, or
# skip a capability check we actually expose). Each pattern below is a claim
# that was true once and is not any more; re-introducing one fails CI.
#
# If a claim here becomes true again (a feature is genuinely removed), delete
# its pattern in the SAME commit that removes the feature.

echo "==> Guard 4: retired capability claims"

# Scanned: markdown and plain text (docs/llms-full.txt is the agent-facing copy
# and carried these claims too). Excluded: changelogs (they record history, which
# legitimately contains the old claim) and generated HTML (rebuilt from the
# markdown by scripts/build-docs-pages.mjs).
#
# THE GAP EXPRESSION IS `[^.]{0,60}`, NOT `[^|]*`. These claims live
# overwhelmingly in MARKDOWN TABLE ROWS — "| Screenshot support | ❌ Roadmap |" —
# where the subject and the claim sit in DIFFERENT CELLS. A `[^|]*` bridge
# cannot cross the cell separator, so it silently matches nothing: the very
# claims this guard exists to catch sailed straight through it.
#
# So the gap must allow `|`. It must NOT be a bare `.*`, which spans a whole
# line and fires on legitimate prose (a sentence that says screenshots DO ship
# and mentions the roadmap 80 characters later). Excluding `.` keeps a match
# inside one sentence, and the {0,60} bound keeps it near its subject.
STALE_PATTERNS=(
  # screenshot: produced on a chrome/chrome_proxy/playwright tier (crw-core OutputFormat::Screenshot)
  'screenshot[^.]{0,60}not (yet )?(supported|implemented|produced)'
  'not (yet )?(supported|implemented|produced)[^.]{0,60}screenshot'
  'screenshot[^.]{0,30}(❌ *)?roadmap'
  # POST /v1/extract exists (routes/v1/mod.rs), and takes a `urls` array
  'no standalone `?/v1/extract'
  '/v1/extract[^.]{0,60}not implemented'
  'multi-url[^.]{0,60}extract[^.]{0,60}not supported'
  'extract[^.]{0,60}multi-url[^.]{0,60}not supported'
  # the TypeScript SDK is published as `crw-sdk`; @fastcrw/sdk never existed
  '@fastcrw/sdk'
  # /v1/search synthesizes an answer when an LLM is available (answer: true)
  'does not synthesize answers'
  'no LLM answer'
  # 1 credit = 1 page, whatever the egress path
  'credit surcharge'
  # bring-your-own-proxy ships on self-host ([proxy] pool + the `proxy` body param)
  'bring[- ]your[- ]own[- ]proxy[^.]{0,60}not (supported|available)'
)

stale=""
for pat in "${STALE_PATTERNS[@]}"; do
  hits=$(
    grep -rniE --include="*.md" --include="*.txt" "${pat}" . 2>/dev/null \
      | grep -vE '(^|/)CHANGELOG\.md:' \
      | grep -vE '^\./docs/docs/changelog\.md:' \
      | grep -vE '^\./(target|node_modules)/' \
      || true
  )
  if [ -n "$hits" ]; then
    stale="${stale}${hits}"$'\n'
  fi
done

if [ -n "$stale" ]; then
  echo "FAIL: a retired capability claim is back in the docs." >&2
  echo "      These features ship today — see scripts/docs-guards.sh Guard 3." >&2
  echo >&2
  echo "$stale" | sed '/^$/d; s/^/  /' >&2
  FAIL=1
else
  echo "ok: no retired capability claims"
fi

# ---------------------------------------------------------------------------

if [ "$FAIL" -ne 0 ]; then
  exit 1
fi

echo "All doc guards passed."
