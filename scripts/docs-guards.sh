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

if [ "$FAIL" -ne 0 ]; then
  exit 1
fi

echo "All doc guards passed."
