#!/usr/bin/env python3
"""Regression tests for the release version-sync guards.

The guards (`check-internal-dep-versions.sh` and `audit_release_please_config.py`)
are load-bearing: they are the only thing standing between a forgotten version
surface and a broken/empty release (the failure class that recurred across
v0.9.0, v0.12.0). A guard that silently passes is worse than no guard, so these
tests pin the load-bearing failure modes — drift, missing config entry,
anti-vacuity, invalid jsonpath — and the green happy path. If a future refactor
weakens a guard, one of these goes red instead of the release.

Each test builds a minimal fixture workspace in a tmpdir and runs the REAL
script entrypoints against it (the bash guard via CHECK_REPO_ROOT, the audit via
cwd), so we test what actually ships, not a reimplementation.

Dependency-free: runs under plain `uv run python scripts/release/test_guards.py`
(no pytest needed — robust in CI without a network install) and is ALSO
collectible by pytest if present, since the test_* functions take no fixtures.
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
GUARD = REPO / "scripts" / "check-internal-dep-versions.sh"
AUDIT = REPO / "scripts" / "release" / "audit_release_please_config.py"

VERSION = "1.0.0"


def _write(p: Path, text: str) -> None:
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(text)


def make_fixture(tmp: Path) -> None:
    """A minimal but valid post-migration workspace: one internal dep (crw-core)
    centralized in [workspace.dependencies], consumed by crw-server via
    inheritance, plus crw-server's allowlisted self dev-dep."""
    _write(
        tmp / "Cargo.toml",
        f"""[workspace]
members = ["crates/crw-core", "crates/crw-server"]

[workspace.package]
version = "{VERSION}"

[workspace.dependencies]
crw-core = {{ path = "crates/crw-core", version = "{VERSION}" }}
""",
    )
    _write(
        tmp / "crates" / "crw-core" / "Cargo.toml",
        """[package]
name = "crw-core"
version.workspace = true

[dependencies]
""",
    )
    _write(
        tmp / "crates" / "crw-server" / "Cargo.toml",
        """[package]
name = "crw-server"
version.workspace = true

[dependencies]
crw-core = { workspace = true }

[dev-dependencies]
crw-server = { path = ".", features = ["test-utils"] }
""",
    )
    _write(
        tmp / "release-please-config.json",
        json.dumps(
            {
                "packages": {
                    ".": {
                        "extra-files": [
                            {
                                "type": "toml",
                                "path": "Cargo.toml",
                                "jsonpath": "$.workspace.package.version",
                            },
                            {
                                "type": "toml",
                                "path": "Cargo.toml",
                                "jsonpath": "$.workspace.dependencies.crw-core.version",
                            },
                        ]
                    }
                }
            },
            indent=2,
        ),
    )


def run_guard(fixture: Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["bash", str(GUARD)],
        env={**os.environ, "CHECK_REPO_ROOT": str(fixture)},
        capture_output=True,
        text=True,
    )


def run_audit(fixture: Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(AUDIT)],
        cwd=str(fixture),
        capture_output=True,
        text=True,
    )


def patch_toml(fixture: Path, old: str, new: str) -> None:
    p = fixture / "Cargo.toml"
    text = p.read_text()
    assert old in text, f"fixture anchor not found: {old!r}"
    p.write_text(text.replace(old, new))


def patch_config(fixture: Path, fn) -> None:
    p = fixture / "release-please-config.json"
    cfg = json.loads(p.read_text())
    fn(cfg)
    p.write_text(json.dumps(cfg, indent=2))


# --- happy path -----------------------------------------------------------


def test_green_baseline(tmp_path):
    make_fixture(tmp_path)
    g = run_guard(tmp_path)
    a = run_audit(tmp_path)
    assert g.returncode == 0, g.stdout + g.stderr
    assert a.returncode == 0, a.stdout + a.stderr


# --- guard failure modes --------------------------------------------------


def test_drift_pin_off_workspace_version(tmp_path):
    """A [workspace.dependencies] pin not equal to the workspace version → red."""
    make_fixture(tmp_path)
    patch_toml(
        tmp_path,
        'crw-core = { path = "crates/crw-core", version = "1.0.0" }',
        'crw-core = { path = "crates/crw-core", version = "0.9.0" }',
    )
    assert run_guard(tmp_path).returncode == 1


def test_anti_vacuity_no_centralized_pins(tmp_path):
    """If the centralized internal pins vanish, the guard must NOT pass vacuously."""
    make_fixture(tmp_path)
    patch_toml(
        tmp_path,
        '\ncrw-core = { path = "crates/crw-core", version = "1.0.0" }\n',
        "\n",
    )
    assert run_guard(tmp_path).returncode == 1


def test_residual_inline_pin_rejected(tmp_path):
    """A member re-declaring an inline internal path pin (instead of inheriting) → red."""
    make_fixture(tmp_path)
    server = tmp_path / "crates" / "crw-server" / "Cargo.toml"
    server.write_text(
        server.read_text().replace(
            "crw-core = { workspace = true }",
            'crw-core = { path = "../crw-core", version = "1.0.0" }',
        )
    )
    assert run_guard(tmp_path).returncode == 1


def test_self_dev_dep_allowlisted(tmp_path):
    """The crw-server self dev-dep (path = '.', no version) must stay allowed."""
    make_fixture(tmp_path)
    # Baseline already contains the self dev-dep; guard must be green.
    assert run_guard(tmp_path).returncode == 0


# --- audit failure modes --------------------------------------------------


def test_missing_config_entry(tmp_path):
    """An internal pin with no release-please-config entry → audit red (it would
    go stale on the next bump — the exact v0.12.0 break)."""
    make_fixture(tmp_path)

    def drop_crw_core(cfg):
        ef = cfg["packages"]["."]["extra-files"]
        cfg["packages"]["."]["extra-files"] = [
            e
            for e in ef
            if e.get("jsonpath") != "$.workspace.dependencies.crw-core.version"
        ]

    patch_config(tmp_path, drop_crw_core)
    assert run_audit(tmp_path).returncode == 1


def test_invalid_jsonpath(tmp_path):
    """A config entry whose jsonpath doesn't resolve to a live field → audit red."""
    make_fixture(tmp_path)

    def add_bad(cfg):
        cfg["packages"]["."]["extra-files"].append(
            {
                "type": "toml",
                "path": "Cargo.toml",
                "jsonpath": "$.workspace.dependencies.crw-nonexistent.version",
            }
        )

    patch_config(tmp_path, add_bad)
    assert run_audit(tmp_path).returncode == 1


def test_audit_anti_vacuity(tmp_path):
    """If no internal pins exist in [workspace.dependencies], the audit's
    completeness check must refuse to pass vacuously."""
    make_fixture(tmp_path)
    patch_toml(
        tmp_path,
        '\ncrw-core = { path = "crates/crw-core", version = "1.0.0" }\n',
        "\n",
    )

    def drop_crw_core(cfg):
        cfg["packages"]["."]["extra-files"] = [
            e
            for e in cfg["packages"]["."]["extra-files"]
            if e.get("jsonpath") != "$.workspace.dependencies.crw-core.version"
        ]

    patch_config(tmp_path, drop_crw_core)
    assert run_audit(tmp_path).returncode == 1


def _run_standalone() -> int:
    """Minimal runner so the suite works without pytest installed.

    pytest's `tmp_path` is a builtin fixture, so under pytest these same
    test_* functions run unchanged; here we supply a fresh tmpdir per test.
    """
    tests = sorted(
        (name, fn)
        for name, fn in globals().items()
        if name.startswith("test_") and callable(fn)
    )
    failures = 0
    for name, fn in tests:
        with tempfile.TemporaryDirectory() as d:
            try:
                fn(Path(d))
                print(f"  PASS {name}")
            except Exception as e:  # noqa: BLE001 — test runner surfaces all
                failures += 1
                print(f"  FAIL {name}: {e}")
    total = len(tests)
    print(f"\n{total - failures}/{total} passed")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(_run_standalone())
