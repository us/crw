#!/usr/bin/env bash
# Common helpers for release scripts.
# Always source with `set -euo pipefail` already enabled in the caller.

# Run a command, capturing combined stdout+stderr without dying on failure.
# Required because `out=$(cmd)` under `set -e` aborts on non-zero rc, hiding
# the output. Use this whenever you need to inspect the failure text.
#
# Usage:
#   out=""; rc=0
#   run_capture out rc cargo publish -p crw-core
#   echo "$out"; echo "rc=$rc"
run_capture() {
  local _outvar="$1"; shift
  local _rcvar="$1"; shift
  local _out _rc
  if _out=$("$@" 2>&1); then _rc=0; else _rc=$?; fi
  printf -v "$_outvar" '%s' "$_out"
  printf -v "$_rcvar" '%s' "$_rc"
}

# Test whether `cargo publish` output indicates the version is already on the
# registry. Used for idempotent re-runs.
is_already_uploaded() {
  # shellcheck disable=SC2016 # backticks are literal in cargo's error message
  printf '%s' "$1" | grep -qE 'already (uploaded|exists)|crate version `[^`]+` is already uploaded'
}

# crates.io API: returns 0 if exact <crate>@<version> exists.
crate_version_present() {
  local crate="$1" version="$2"
  curl -fsSL -H "User-Agent: crw-release" \
    "https://crates.io/api/v1/crates/${crate}/${version}" 2>/dev/null \
    | jq -e --arg v "$version" '.version.num == $v' >/dev/null 2>&1
}

# Sparse-index path for a crate name, per the registry index protocol:
# 1 char -> `1/<name>`, 2 -> `2/<name>`, 3 -> `3/<first>/<name>`,
# 4+ -> `<first two>/<next two>/<name>`. Names are lowercased in the index.
crate_index_path() {
  local name
  name=$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')
  case ${#name} in
    1) printf '1/%s' "$name" ;;
    2) printf '2/%s' "$name" ;;
    3) printf '3/%s/%s' "${name:0:1}" "$name" ;;
    *) printf '%s/%s/%s' "${name:0:2}" "${name:2:2}" "$name" ;;
  esac
}

# crates.io cksum for an existing version (sha256 of the .crate file), read from
# the sparse index rather than the v1 API.
#
# The v1 API's `/crates/<name>/<version>` response no longer carries
# `version.cksum` (verified 2026-09-10: absent for every crate queried), so the
# old lookup silently returned an empty string. Because the caller compared that
# empty value against the local sha directly, every idempotent re-publish failed
# as a "content mismatch" and told the maintainer to bump the version. The
# sparse index is the registry protocol itself and is where the checksum is
# authoritative.
#
# Prints the checksum and returns 0 on success; prints nothing and returns 1
# when the index or the version cannot be read, so the caller can tell "unknown"
# apart from "different".
crate_version_cksum() {
  local crate="$1" version="$2" body sum
  body=$(curl -fsSL --retry 3 --retry-connrefused -H "User-Agent: crw-release" \
    "https://index.crates.io/$(crate_index_path "$crate")" 2>/dev/null) || return 1
  sum=$(printf '%s' "$body" | jq -r --arg v "$version" 'select(.vers == $v) | .cksum' 2>/dev/null | head -1)
  [ -n "$sum" ] || return 1
  printf '%s' "$sum"
}

# Parse a workspace member's local version from its Cargo.toml.
# Falls back to root workspace.package.version (members typically inherit).
crate_local_version() {
  local crate="$1" workspace_dir="${2:-.}"
  local manifest="${workspace_dir}/crates/${crate}/Cargo.toml"
  local v
  v=$(grep -E '^version\s*=' "$manifest" | head -1 | sed -E 's/.*"([^"]+)".*/\1/')
  if [ -z "$v" ]; then
    v=$(grep -E '^version\s*=' "${workspace_dir}/Cargo.toml" | head -1 | sed -E 's/.*"([^"]+)".*/\1/')
  fi
  [ -n "$v" ] || { echo "::error::cannot determine version for $crate" >&2; return 1; }
  printf '%s' "$v"
}

# Annotation helpers — visible in GitHub Actions logs.
notice() { printf '::notice::%s\n' "$*"; }
warn()   { printf '::warning::%s\n' "$*"; }
err()    { printf '::error::%s\n' "$*" >&2; }
die()    { err "$*"; exit 1; }
group()    { printf '::group::%s\n' "$*"; }
endgroup() { printf '::endgroup::\n'; }
