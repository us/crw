#!/usr/bin/env bash
# Orchestrates every per-registry verification and writes a markdown audit
# log as a workflow artifact.
#
# Usage: verify_publish.sh <version> [--correlation-id ID] [--out PATH]
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$SCRIPT_DIR/lib.sh"

v=""; corr=""; out=""
while [ $# -gt 0 ]; do
  case "$1" in
    --correlation-id) corr="$2"; shift 2 ;;
    --out)            out="$2";  shift 2 ;;
    *) if [ -z "$v" ]; then v="$1"; fi; shift ;;
  esac
done
[ -n "$v" ] || die "usage: verify_publish.sh <version> [--correlation-id ID] [--out PATH]"
out="${out:-release-audit-${v}.md}"

declare -a results

run_check() {
  local label="$1"; shift
  if "$@" >/tmp/verify-out 2>&1; then
    results+=("| $label | ✓ |")
  else
    results+=("| $label | ❌ |")
    cat /tmp/verify-out >&2
  fi
}

run_check "crates.io"   "$SCRIPT_DIR/verify_crates.sh"        "$v"
run_check "PyPI"        "$SCRIPT_DIR/verify_pypi.sh"          "$v"
# npm can be temporarily disabled (SKIP_NPM=true) when the npm token can't
# publish (e.g. 2FA-bypass token not yet configured). Skipping is explicit in
# the audit — not a silent pass. Re-enable by clearing the SKIP_NPM repo var.
if [ "${SKIP_NPM:-}" = "true" ]; then
  results+=("| npm | skipped (SKIP_NPM) |")
else
  run_check "npm"       "$SCRIPT_DIR/verify_npm.sh"           "$v"
fi
run_check "Docker GHCR" "$SCRIPT_DIR/verify_docker.sh"        "$v"
run_check "MCP registry""$SCRIPT_DIR/verify_mcp_registry.sh"  "$v"
if [ -n "$corr" ]; then
  run_check "APT/Homebrew" "$SCRIPT_DIR/verify_apt_homebrew.sh" "$v" "$corr"
else
  results+=("| APT/Homebrew | skipped (no correlation_id) |")
fi

{
  printf '# Release v%s audit\n\n' "$v"
  printf '| Target | Status |\n'
  printf '|---|---|\n'
  printf '%s\n' "${results[@]}"
} > "$out"

cat "$out"
if printf '%s\n' "${results[@]}" | grep -q '❌'; then
  err "verify_publish: at least one target failed; see $out"
  exit 1
fi
notice "verify_publish OK; audit at $out"
