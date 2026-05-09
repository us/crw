#!/usr/bin/env bash
# Verify the npm main + 6 platform packages and that the main package's
# optionalDependencies all pin to the same version.
#
# Layered checks (codex review v3 C2 + v4 C10):
#   1. Existence of every package@version
#   2. optionalDependencies map exact-version assertion (catches the 0.6.0
#      regression where main was 0.6.0 but optional pins were 0.3.5)
#   3. Install smoke (current OS executable resolution check)
#
# Usage: verify_npm.sh <version>
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$SCRIPT_DIR/lib.sh"

v="${1:?version required}"
fail=0

# 1. Existence
for p in crw-mcp crw-mcp-darwin-x64 crw-mcp-darwin-arm64 \
         crw-mcp-linux-x64 crw-mcp-linux-arm64 \
         crw-mcp-win32-x64 crw-mcp-win32-arm64; do
  actual=$(npm view "$p@$v" version 2>/dev/null || echo "MISSING")
  if [ "$actual" = "$v" ]; then
    printf '✓ %s@%s\n' "$p" "$v"
  else
    printf '❌ %s@%s (got %s)\n' "$p" "$v" "$actual"
    fail=1
  fi
done

# 2. optionalDependencies pin assertion — catches stale pin sneak-through
deps_json=$(npm view "crw-mcp@$v" optionalDependencies --json 2>/dev/null || echo '{}')
for p in crw-mcp-darwin-x64 crw-mcp-darwin-arm64 \
         crw-mcp-linux-x64 crw-mcp-linux-arm64 \
         crw-mcp-win32-x64 crw-mcp-win32-arm64; do
  pin=$(printf '%s' "$deps_json" | jq -r --arg p "$p" '.[$p] // "MISSING"')
  # Accept exact, ^v, ~v
  if [ "$pin" = "$v" ] || [ "$pin" = "^$v" ] || [ "$pin" = "~$v" ]; then
    printf '✓ optionalDependencies.%s = %s\n' "$p" "$pin"
  else
    printf '❌ optionalDependencies.%s = %s (expected %s)\n' "$p" "$pin" "$v"
    fail=1
  fi
done

# 3. Install smoke
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
(cd "$tmp" && npm init -y >/dev/null && npm install --silent "crw-mcp@$v" >/dev/null 2>&1) \
  || { err "npm install crw-mcp@$v failed"; fail=1; }
# shellcheck disable=SC2015
resolved=$(cd "$tmp" && npm ls --all 2>/dev/null | grep -E "crw-mcp-(darwin|linux|win32)" | head -1 || true)
if [ -n "$resolved" ] && ! printf '%s' "$resolved" | grep -q "@$v"; then
  err "platform pkg resolved to wrong version: $resolved"
  fail=1
fi

exit "$fail"
