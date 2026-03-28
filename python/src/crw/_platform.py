"""Platform detection and GitHub Release asset mapping."""

import platform
import sys

# Maps (system, machine) to GitHub Release asset filename
PLATFORM_MAP: dict[tuple[str, str], str] = {
    ("Darwin", "arm64"): "crw-mcp-darwin-arm64.tar.gz",
    ("Darwin", "x86_64"): "crw-mcp-darwin-x64.tar.gz",
    ("Linux", "x86_64"): "crw-mcp-linux-x64.tar.gz",
    ("Linux", "aarch64"): "crw-mcp-linux-arm64.tar.gz",
    ("Windows", "AMD64"): "crw-mcp-win32-x64.zip",
    ("Windows", "ARM64"): "crw-mcp-win32-arm64.zip",
}

BINARY_NAME = "crw-mcp.exe" if sys.platform == "win32" else "crw-mcp"


def get_asset_name() -> str | None:
    """Return the GitHub Release asset filename for the current platform."""
    key = (platform.system(), platform.machine())
    return PLATFORM_MAP.get(key)
