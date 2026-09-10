#!/usr/bin/env bash
# Publish a single crate to crates.io, idempotent.
# Treats "already uploaded" as success ONLY when the local .crate sha256
# matches crates.io cksum (otherwise we'd silently bless a mismatched prior
# upload — see codex review v4 W10).
#
# Usage: publish_crate.sh <crate> <version> [--source-dir DIR]
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$SCRIPT_DIR/lib.sh"

crate=""; version=""; source_dir="."
while [ $# -gt 0 ]; do
  case "$1" in
    --source-dir) source_dir="$2"; shift 2 ;;
    *) if [ -z "$crate" ]; then crate="$1"; elif [ -z "$version" ]; then version="$1"; fi; shift ;;
  esac
done
[ -n "$crate" ] && [ -n "$version" ] || die "usage: publish_crate.sh <crate> <version> [--source-dir DIR]"

cd "$source_dir"

group "cargo publish -p $crate"
out=""; rc=0
run_capture out rc cargo publish -p "$crate"
printf '%s\n' "$out"
endgroup

if [ "$rc" -eq 0 ]; then
  notice "$crate@$version published"
  exit 0
fi

if ! is_already_uploaded "$out"; then
  err "cargo publish $crate failed (rc=$rc)"
  exit "$rc"
fi

# "already uploaded" — verify content matches before treating as success.
warn "$crate@$version already on crates.io; verifying cksum"
if ! crate_version_present "$crate" "$version"; then
  die "$crate@$version reported already-uploaded but API says not found"
fi

# Package locally for cksum compare. --no-verify avoids re-resolving deps.
group "cargo package -p $crate (cksum check)"
cargo package -p "$crate" --allow-dirty --no-verify >/dev/null
endgroup
local_path="target/package/${crate}-${version}.crate"
[ -f "$local_path" ] || die "expected $local_path after cargo package"
local_sha=$(sha256sum "$local_path" | awk '{print $1}')
# An unreadable checksum is NOT evidence of a mismatch. Keeping the two apart
# matters: the old code compared the local sha against whatever the lookup
# returned, so an empty result failed the release with "content mismatch" and
# told the maintainer to bump the version, for content that was in fact
# identical. Say what is actually known instead.
remote_sha=""
if ! remote_sha=$(crate_version_cksum "$crate" "$version"); then
  err "$crate@$version is already on crates.io but its checksum could not be read"
  err "from the sparse index, so this run cannot confirm the upload matches."
  err "Re-run once the index is reachable; do NOT bump the version on this alone."
  exit 1
fi
if [ "$local_sha" != "$remote_sha" ]; then
  err "$crate@$version content mismatch — local=$local_sha remote=$remote_sha"
  err "crates.io is immutable; cannot republish. Bump the version."
  exit 1
fi
notice "$crate@$version cksum matches; idempotent skip"
