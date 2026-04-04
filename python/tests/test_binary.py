"""Unit tests for binary resolution and platform detection."""

from __future__ import annotations

import os
from pathlib import Path
from unittest.mock import patch

import pytest

from crw._binary import ensure_binary
from crw._platform import PLATFORM_MAP, get_asset_name
from crw.exceptions import CrwBinaryNotFoundError


@pytest.mark.unit
class TestEnsureBinaryEnv:
    def test_ensure_binary_env_override(self, tmp_path: Path) -> None:
        """CRW_BINARY pointing to a real file should be returned directly."""
        fake_binary = tmp_path / "crw-mcp"
        fake_binary.write_text("#!/bin/sh\necho hi")
        fake_binary.chmod(0o755)

        with patch.dict(os.environ, {"CRW_BINARY": str(fake_binary)}):
            result = ensure_binary()
            assert result == fake_binary

    def test_ensure_binary_env_override_missing(self) -> None:
        """CRW_BINARY pointing to a nonexistent path should raise."""
        with patch.dict(os.environ, {"CRW_BINARY": "/no/such/binary"}):
            with pytest.raises(CrwBinaryNotFoundError, match="does not exist"):
                ensure_binary()


@pytest.mark.unit
class TestGetAssetName:
    def test_get_asset_name_returns_string(self) -> None:
        result = get_asset_name()
        # On a supported platform this is a string; on unsupported it's None
        if result is not None:
            assert isinstance(result, str)
            assert "crw-mcp" in result


@pytest.mark.unit
class TestPlatformMap:
    def test_platform_map_coverage(self) -> None:
        """All six expected platform entries must exist."""
        expected_keys = [
            ("Darwin", "arm64"),
            ("Darwin", "x86_64"),
            ("Linux", "x86_64"),
            ("Linux", "aarch64"),
            ("Windows", "AMD64"),
            ("Windows", "ARM64"),
        ]
        assert len(PLATFORM_MAP) == 6
        for key in expected_keys:
            assert key in PLATFORM_MAP, f"Missing platform entry: {key}"
            assert isinstance(PLATFORM_MAP[key], str)
