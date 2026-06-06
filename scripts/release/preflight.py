#!/usr/bin/env python3
"""Pre-tag publishability checks.

Runs in PR CI to catch publish-breaking changes before a release tag is
cut. Combines several guards that historically failed silently:

  1. Every workspace member is classified in release_manifest.toml as either
     published (in a tier) or unpublished — no silent third state.

  2. Each crate's Cargo.toml `publish` flag matches the manifest. Source of
     truth is parsed directly from Cargo.toml (cargo metadata's representation
     of `publish = false` is unreliable, see codex review v3 C4).

  3. Published crates' default features must not transitively pull in
     unpublished crates. Walks dep aliases and recurses into internal
     workspace crates (codex review v3 C6, v5 W14).

  4. Every published crate's path/git deps have an explicit version field
     (Cargo refuses to publish otherwise, but the failure mode is opaque).

  5. Workspace version is consistent across all version surfaces:
     - Cargo.toml workspace.package.version
     - pyproject.toml project.version
     - npm/crw-mcp/package.json version + every optionalDependencies pin
     - npm/crw-mcp-*/package.json version (per platform)
     - server.json (if present) version
     (codex review v4 S3.)

  6. release-please-config.json extra-files entries are valid (delegated to
     audit_release_please_config.py).

Exits non-zero on any failure with `::error::` annotations for GitHub Actions.
"""
from __future__ import annotations

import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path


# ---------- TOML manifest parsing ----------

def load_manifest(path: Path = Path("scripts/release/release_manifest.toml")) -> dict:
    return tomllib.loads(path.read_text())


def is_published(cargo_toml_path: Path) -> bool:
    """Return True iff this Cargo.toml allows publishing to the default registry."""
    pkg = tomllib.loads(cargo_toml_path.read_text()).get("package", {})
    if "publish" not in pkg:
        return True  # default = publish to crates.io
    val = pkg["publish"]
    if val is False:
        return False
    if isinstance(val, list) and len(val) == 0:
        return False  # empty registry list = effectively unpublished
    return True


# ---------- cargo metadata ----------

def cargo_metadata() -> dict:
    out = subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version=1"]
    )
    return json.loads(out)


# ---------- feature graph walker ----------

def _build_dep_alias_map(pkg: dict) -> dict[str, str]:
    """Map feature-entry alias name -> real package name."""
    m: dict[str, str] = {}
    for d in pkg.get("dependencies", []):
        alias = d.get("rename") or d["name"]
        m[alias] = d["name"]
    return m


def _one_hop_default_pull(pkg: dict) -> set[str]:
    """Resolve `default` feature once; return real package names pulled in."""
    alias_map = _build_dep_alias_map(pkg)
    feats = pkg.get("features", {})
    raw: set[str] = set()
    seen: set[str] = set()

    def expand(name: str) -> None:
        if name in seen:
            return
        seen.add(name)
        for entry in feats.get(name, []):
            if entry.startswith("dep:"):
                raw.add(entry[4:])
            elif "/" in entry:
                raw.add(entry.split("/", 1)[0].rstrip("?"))
            else:
                expand(entry)

    expand("default")

    # Always-pulled (non-optional) deps count toward the chain.
    for d in pkg.get("dependencies", []):
        if d.get("kind") in (None, "build") and not d.get("optional", False):
            raw.add(d["name"])

    return {alias_map.get(a, a) for a in raw}


def transitive_default_chain(
    pkg: dict, all_pkgs_by_name: dict[str, dict], visited: set[str] | None = None
) -> set[str]:
    visited = visited if visited is not None else set()
    if pkg["name"] in visited:
        return set()
    visited.add(pkg["name"])
    direct = _one_hop_default_pull(pkg)
    result = set(direct)
    for dep_name in direct:
        if dep_name in all_pkgs_by_name:
            result |= transitive_default_chain(
                all_pkgs_by_name[dep_name], all_pkgs_by_name, visited
            )
    return result


# ---------- version surface check ----------

def workspace_version() -> str:
    data = tomllib.loads(Path("Cargo.toml").read_text())
    return data["workspace"]["package"]["version"]


def collect_version_surfaces(ws_version: str) -> list[tuple[str, str, str]]:
    """Return [(surface_label, expected, actual)] for every version pin.

    Includes optional surfaces (skipped silently if file missing).
    """
    rows: list[tuple[str, str, str]] = []

    # pyproject.toml
    py = Path("python/pyproject.toml")
    if py.exists():
        d = tomllib.loads(py.read_text())
        rows.append((str(py), ws_version, d.get("project", {}).get("version", "MISSING")))

    # npm platform packages, discovered by globbing `npm/crw-mcp-*/package.json`
    # (the main package `npm/crw-mcp` has no trailing `-` so it is excluded).
    # Self-deriving: adding a platform needs NO edit here — the old hardcoded
    # 6-tuples meant a 7th platform was silently unchecked.
    platform_pkgs = sorted(Path("npm").glob("crw-mcp-*/package.json"))
    disk_platforms = {p.parent.name for p in platform_pkgs}

    # npm main package: version + every internal optionalDependencies pin, with
    # the platform key set read from the manifest itself.
    npm_main = Path("npm/crw-mcp/package.json")
    opt_platforms: set[str] = set()
    if npm_main.exists():
        d = json.loads(npm_main.read_text())
        rows.append((str(npm_main) + ":version", ws_version, d.get("version", "MISSING")))
        for dep, actual in sorted(d.get("optionalDependencies", {}).items()):
            if not dep.startswith("crw-mcp-"):
                continue
            opt_platforms.add(dep)
            # Accept exact, ^v, ~v.
            if actual in (ws_version, f"^{ws_version}", f"~{ws_version}"):
                continue
            rows.append((f"{npm_main}:optionalDependencies.{dep}", ws_version, actual))

        # Cross-check: every platform package on disk must be listed in
        # optionalDependencies (and vice-versa), so a new platform can't be
        # half-wired and silently skipped.
        for missing in sorted(disk_platforms - opt_platforms):
            rows.append((
                f"{npm_main}:optionalDependencies.{missing}", ws_version,
                "MISSING (platform package exists on disk but not in optionalDependencies)",
            ))
        for extra in sorted(opt_platforms - disk_platforms):
            rows.append((
                f"{npm_main}:optionalDependencies.{extra}", "a platform package dir",
                "MISSING (listed in optionalDependencies but no npm/<plat>/package.json)",
            ))

    for pkg_json in platform_pkgs:
        d = json.loads(pkg_json.read_text())
        rows.append((str(pkg_json) + ":version", ws_version, d.get("version", "MISSING")))

    # server.json (MCP registry)
    sj = Path("server.json")
    if sj.exists():
        d = json.loads(sj.read_text())
        rows.append((str(sj) + ":version", ws_version, d.get("version", "MISSING")))

    return rows


# ---------- main ----------

def main() -> int:
    errors: list[str] = []
    manifest = load_manifest()
    published = {c for tier in manifest["tiers"] for c in tier["crates"]}
    unpublished = set(manifest.get("unpublished", {}).get("crates", []))

    if published & unpublished:
        errors.append(f"manifest: crates in both tiers and unpublished: {published & unpublished}")

    meta = cargo_metadata()
    members = {p["name"] for p in meta["packages"]}
    by_name = {p["name"]: p for p in meta["packages"]}

    # 1. Classification coverage
    unaccounted = members - published - unpublished
    if unaccounted:
        errors.append(f"workspace members not in release_manifest.toml: {sorted(unaccounted)}")

    # 2. Cargo.toml publish flag matches manifest
    for member in members:
        cargo_path = Path("crates") / member / "Cargo.toml"
        if not cargo_path.exists():
            errors.append(f"{member}: Cargo.toml not found at {cargo_path}")
            continue
        declared = is_published(cargo_path)
        expected = member in published
        if declared and member in unpublished:
            errors.append(
                f"{member}: Cargo.toml allows publish but manifest marks unpublished. "
                f"Add `publish = false` to {cargo_path}."
            )
        elif not declared and member in published:
            errors.append(
                f"{member}: Cargo.toml has publish=false but manifest marks published. "
                f"Remove `publish = false` from {cargo_path} or move crate to [unpublished]."
            )

    # 3. Default features must not pull unpublished crates
    for pkg in meta["packages"]:
        if pkg["name"] not in published:
            continue
        chain = transitive_default_chain(pkg, by_name)
        bad = chain & unpublished
        if bad:
            errors.append(
                f"{pkg['name']}: default features transitively pull unpublished {sorted(bad)}; "
                f"either publish those crates or remove from defaults."
            )

    # 3b. Tier topology: every internal dep must publish in an EARLIER tier.
    #     Within a tier crates are assumed independent — the driver waits for
    #     crates.io propagation only at tier boundaries, not between same-tier
    #     crates. A same-tier (or later) dep means a crate tries to resolve a
    #     sibling that isn't on the index yet, which aborts the crates.io
    #     publish from that crate onward (this skipped renderer..mcp on 0.13.0).
    tier_of = {c: t["order"] for t in manifest["tiers"] for c in t["crates"]}
    for pkg in meta["packages"]:
        name = pkg["name"]
        if name not in tier_of:
            continue
        for dep in pkg.get("dependencies", []):
            if dep.get("kind") not in (None, "build"):
                continue
            dn = dep["name"]
            if dn in tier_of and tier_of[dn] >= tier_of[name]:
                errors.append(
                    f"tier ordering: {name} (tier {tier_of[name]}) depends on "
                    f"{dn} (tier {tier_of[dn]}) — a dependency must publish in an "
                    f"earlier tier; move {dn} up or {name} down in "
                    f"release_manifest.toml."
                )

    # 3c. No published crate may include_str!/include_bytes! a path that escapes
    #     its own directory — cargo publish only packages files INSIDE the crate,
    #     so an out-of-crate include compiles locally but breaks the publish
    #     verify build (this kept crw-server unpublishable: it embedded the
    #     workspace-root docs/openapi.json via ../../../../).
    inc_re = re.compile(r"include_(?:str|bytes)!\s*\(\s*\"([^\"]+)\"")
    for member in sorted(published):
        crate_dir = (Path("crates") / member).resolve()
        if not crate_dir.is_dir():
            continue
        for rs in sorted(crate_dir.rglob("*.rs")):
            for inc in inc_re.findall(rs.read_text()):
                target = (rs.parent / inc).resolve()
                try:
                    target.relative_to(crate_dir)
                except ValueError:
                    errors.append(
                        f"{rs.relative_to(crate_dir.parent.parent)}: include of "
                        f"'{inc}' reaches outside the crate — it won't be in the "
                        f"published .crate tarball; move the file inside "
                        f"crates/{member}/."
                    )

    # 3d. Duplicated spec copies kept for publishability must not drift.
    for a, b in [
        ("docs/openapi.json", "crates/crw-server/openapi/openapi.json"),
        ("docs/openapi-3.0.json", "crates/crw-server/openapi/openapi-3.0.json"),
    ]:
        pa, pb = Path(a), Path(b)
        if pa.exists() and pb.exists() and pa.read_bytes() != pb.read_bytes():
            errors.append(
                f"{a} and {b} have drifted — the crate copy (published) must stay "
                f"byte-identical to the docs-site copy. Re-sync them."
            )

    # 4. Path/git deps need version field
    for pkg in meta["packages"]:
        if pkg["name"] not in published:
            continue
        for dep in pkg.get("dependencies", []):
            if dep.get("kind") not in (None, "build"):
                continue
            src = dep.get("source")
            req = dep.get("req", "")
            if src is None or src.startswith("git+"):
                if not req or req == "*":
                    errors.append(
                        f"{pkg['name']} → {dep['name']}: "
                        f"path/git dep without version field (cargo will refuse to publish)."
                    )

    # 5. Version surface consistency
    ws_v = workspace_version()
    for label, expected, actual in collect_version_surfaces(ws_v):
        if actual != expected:
            errors.append(f"{label}: expected {expected}, found {actual}")

    if errors:
        print("::error::preflight failed", file=sys.stderr)
        for e in errors:
            print(f"  - {e}", file=sys.stderr)
        return 1
    print(f"::notice::preflight OK (version {ws_v}, {len(published)} published crates)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
