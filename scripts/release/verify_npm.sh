#!/usr/bin/env bash
# Verify the npm main + 4 platform packages (darwin/linux) and that the main
# package's optionalDependencies all pin to the same version. The win32
# platform-package names are npm-security-held (0.0.1-security, owned by
# npm-support), so Windows is served by the launcher's GitHub-download
# fallback instead of a prebuilt package — nothing to verify on npm here.
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

# 1. Existence — poll, since npm registry propagation can lag several seconds
#    after publish (same race already fixed for the SDK in verify_npm_sdk.sh).
#    `crw-mcp` is checked first and is the one that races; once it propagates,
#    the optionalDeps read and install smoke below succeed too.
for p in crw-mcp crw-mcp-darwin-x64 crw-mcp-darwin-arm64 \
         crw-mcp-linux-x64 crw-mcp-linux-arm64; do
  actual="MISSING"
  for _ in 1 2 3 4 5 6; do
    actual=$(npm view "$p@$v" version 2>/dev/null || echo "MISSING")
    [ "$actual" = "$v" ] && break
    sleep 10
  done
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
         crw-mcp-linux-x64 crw-mcp-linux-arm64; do
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
resolved=$(cd "$tmp" && npm ls --all 2>/dev/null | grep -E "crw-mcp-(darwin|linux)" | head -1 || true)
if [ -n "$resolved" ] && ! printf '%s' "$resolved" | grep -q "@$v"; then
  err "platform pkg resolved to wrong version: $resolved"
  fail=1
fi

exit "$fail"
