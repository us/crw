"""Download, cache, and locate the crw-mcp binary."""

from __future__ import annotations

import os
import stat
import tarfile
import zipfile
from io import BytesIO
from pathlib import Path
from urllib.request import Request, urlopen

from platformdirs import user_cache_dir

from crw._platform import BINARY_NAME, get_asset_name
from crw.exceptions import CrwBinaryNotFoundError

try:
    from importlib.metadata import version as _pkg_version
    BINARY_VERSION = _pkg_version("crw")
except Exception:
    BINARY_VERSION = "0.2.1"  # fallback for development
GITHUB_REPO = "us/crw"
DOWNLOAD_URL = f"https://github.com/{GITHUB_REPO}/releases/download/v{BINARY_VERSION}"


def _cache_dir() -> Path:
    return Path(user_cache_dir("crw")) / f"v{BINARY_VERSION}"


def _cached_binary() -> Path | None:
    """Return path to cached binary if it exists and is executable."""
    path = _cache_dir() / BINARY_NAME
    if path.is_file() and os.access(path, os.X_OK):
        return path
    return None


def _download_binary() -> Path:
    """Download the binary from GitHub Releases and cache it."""
    asset = get_asset_name()
    if asset is None:
        raise CrwBinaryNotFoundError(
            f"No prebuilt binary for this platform. "
            f"Install from source: cargo install crw-mcp"
        )

    url = f"{DOWNLOAD_URL}/{asset}"
    cache = _cache_dir()
    cache.mkdir(parents=True, exist_ok=True)

    req = Request(url, headers={"User-Agent": f"crw-python/{BINARY_VERSION}"})
    with urlopen(req, timeout=120) as resp:
        data = resp.read()

    # Extract binary from archive
    if asset.endswith(".tar.gz"):
        with tarfile.open(fileobj=BytesIO(data), mode="r:gz") as tar:
            for member in tar.getmembers():
                if member.name.endswith("crw-mcp"):
                    member.name = BINARY_NAME
                    tar.extract(member, path=cache)
                    break
            else:
                raise CrwBinaryNotFoundError(f"crw-mcp not found in {asset}")
    elif asset.endswith(".zip"):
        with zipfile.ZipFile(BytesIO(data)) as zf:
            for name in zf.namelist():
                if name.endswith("crw-mcp.exe") or name.endswith("crw-mcp"):
                    target = cache / BINARY_NAME
                    target.write_bytes(zf.read(name))
                    break
            else:
                raise CrwBinaryNotFoundError(f"crw-mcp not found in {asset}")

    binary = cache / BINARY_NAME
    binary.chmod(binary.stat().st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)
    return binary


def _is_native_binary(path: Path) -> bool:
    """Check if a file is a native binary (not a Python/shell script)."""
    try:
        with open(path, "rb") as f:
            header = f.read(4)
        # ELF, Mach-O, or PE magic bytes
        return header[:4] in (b"\x7fELF", b"\xcf\xfa\xed\xfe", b"\xce\xfa\xed\xfe", b"MZ\x90\x00", b"MZ\x00\x00")
    except OSError:
        return False


def ensure_binary() -> Path:
    """Return path to crw-mcp binary, downloading if necessary."""
    # 1. Check CRW_BINARY env override
    env_path = os.environ.get("CRW_BINARY")
    if env_path:
        p = Path(env_path)
        if p.is_file():
            return p
        raise CrwBinaryNotFoundError(f"CRW_BINARY={env_path} does not exist")

    # 2. Check if crw-mcp native binary is on PATH (e.g. cargo install)
    # Search all PATH entries, skipping our Python wrapper
    for directory in os.environ.get("PATH", "").split(os.pathsep):
        candidate = Path(directory) / BINARY_NAME
        if candidate.is_file() and _is_native_binary(candidate):
            return candidate

    # 3. Check cache
    cached = _cached_binary()
    if cached:
        return cached

    # 4. Download
    return _download_binary()
