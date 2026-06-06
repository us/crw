#!/usr/bin/env bash
# Mechanical guard against the stale-internal-dependency-version release break.
#
# Every internal crate is published to crates.io, so its sibling dependencies
# carry a `version = "X"` compatibility assertion. Those versions are now
# centralized in the root `[workspace.dependencies]` table (members inherit via
# `{ workspace = true }`), so there is ONE place per internal crate to keep in
# sync with the unified workspace version (`[workspace.package].version`). If a
# release bumps the workspace version but leaves a `[workspace.dependencies]`
# entry behind, cargo can no longer resolve the path crate (`^old` excludes the
# new version) and every publish job in the Release workflow is skipped —
# shipping an empty tag.
#
# This is exactly what happened to v0.9.0 (crw-cli pins stayed 0.8.1) and again
# to v0.12.0 (crw-diff -> crw-core stayed 0.11.0). The heavy `cargo build` CI
# step did not block the release-please PR (Cargo.lock cache short-circuited
# resolution), so this dedicated, cache-proof invariant check turns a
# catastrophic post-tag failure into a pre-merge red X.
#
# Three invariants, all derived from `[workspace] members` (so the guard never
# needs editing when crates are added or removed):
#   1. every internal pin in [workspace.dependencies] == the workspace version;
#   2. anti-vacuity — there MUST be centralized internal pins (an empty scan is
#      itself an error, so the migration to inheritance can't silently disable
#      this guard);
#   3. residual scan — no member manifest may re-declare an inline `path` pin
#      for an internal crate (it must inherit), with one allowlisted exception:
#      crw-server's self dev-dependency (`path = "."`, no version).
#
# Portable: bash + python3 (present on ubuntu-latest and macOS).
set -euo pipefail

# Repo root defaults to this script's parent, but tests override it with a
# fixture dir (scripts/release/test_guards.py) to exercise failure modes.
cd "${CHECK_REPO_ROOT:-$(dirname "$0")/..}"

python3 - <<'PY'
import sys, tomllib
from pathlib import Path

root = tomllib.loads(Path("Cargo.toml").read_text())
ws = root.get("workspace", {})

ws_version = ws.get("package", {}).get("version")
if not ws_version:
    print("error: could not find [workspace.package] version in root Cargo.toml")
    sys.exit(2)

members = ws.get("members")
if not members:
    print("error: could not find [workspace] members in root Cargo.toml")
    sys.exit(2)
# Internal crate names = basenames of [workspace] members.
internal = {Path(m).name for m in members}

problems = []

# (1) Centralized internal pins in root [workspace.dependencies] must equal the
#     workspace version.
central = {
    name: spec["version"]
    for name, spec in ws.get("dependencies", {}).items()
    if name in internal
    and isinstance(spec, dict)
    and "path" in spec
    and "version" in spec
}
for name, ver in sorted(central.items()):
    if ver != ws_version:
        problems.append(
            f'[workspace.dependencies] {name}: version "{ver}" '
            f'must equal workspace version "{ws_version}"'
        )

# (2) Anti-vacuity: an empty centralized set means the guard would check nothing.
if not central:
    problems.append(
        "no internal crate pins found in root [workspace.dependencies] — the "
        "guard would pass vacuously; refusing to pass (did the workspace-deps "
        "migration regress?)"
    )

# (3) Residual scan: members must inherit internal deps via `{ workspace = true }`,
#     never re-declare an inline `path` pin (which escapes centralized bumping).
#     Sole allowed exception: a crate's self dev-dependency (path = ".").
ALLOWLIST = {("crates/crw-server/Cargo.toml", "crw-server")}
_TABLES = ("dependencies", "dev-dependencies", "build-dependencies")
for cargo in sorted(Path("crates").glob("*/Cargo.toml")):
    data = tomllib.loads(cargo.read_text())
    for table in _TABLES:
        for name, spec in data.get(table, {}).items():
            if name not in internal:
                continue
            if isinstance(spec, dict) and "path" in spec:
                if (cargo.as_posix(), name) in ALLOWLIST:
                    continue
                problems.append(
                    f"{cargo.as_posix()}: {name} re-declares an inline `path` pin "
                    f"in [{table}]; internal deps must inherit via "
                    f"`{{ workspace = true }}` so release-please bumps them centrally"
                )

if problems:
    print(f"❌ internal dependency version guard failed (workspace version is {ws_version}):\n")
    for p in problems:
        print(f"  {p}")
    print(
        "\nFix: internal crate versions live ONLY in root [workspace.dependencies]; "
        "set each to the workspace version and add the matching "
        "release-please-config.json extra-files entry "
        "($.workspace.dependencies.<crate>.version)."
    )
    sys.exit(1)

print(
    f"✅ {len(central)} centralized internal pins all match workspace version "
    f"{ws_version}; no stray inline pins"
)
PY
