"""Unit tests for binary resolution and platform detection."""

from __future__ import annotations

import hashlib
import importlib.metadata
import io
import os
import tarfile
from pathlib import Path
from unittest.mock import patch
from urllib.error import HTTPError, URLError

import pytest

from crw._binary import (
    BINARY_NAME,
    _download_binary,
    _release_tag,
    ensure_binary,
)
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


def _targz(content: bytes = b"#!/bin/sh\nexit 0\n") -> bytes:
    """A minimal release archive containing a crw-mcp entry."""
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tar:
        info = tarfile.TarInfo("crw-mcp")
        info.size = len(content)
        info.mode = 0o755
        tar.addfile(info, io.BytesIO(content))
    return buf.getvalue()


@pytest.mark.unit
class TestDownloadVerification:
    """The digest check must fail closed. No fallback, no unverified binary.

    This is the test that enforces the thesis of the hardening work: if it can
    be made to pass while a mismatched archive still gets installed, the
    verification is decorative.
    """

    ASSET = "crw-mcp-linux-x64.tar.gz"

    def _run(self, tmp_path: Path, sums: bytes | Exception, archive: bytes):
        responses: list[bytes | Exception] = [sums, archive]

        def fake_urlopen(_req, **_kw):
            item = responses.pop(0)
            if isinstance(item, Exception):
                raise item
            return io.BytesIO(item)

        with (
            patch("crw._binary.get_asset_name", return_value=self.ASSET),
            patch("crw._binary.urlopen", side_effect=fake_urlopen),
            patch("crw._binary.user_cache_dir", return_value=str(tmp_path)),
        ):
            return _download_binary("v9.9.9")

    def test_mismatch_raises_and_installs_nothing(self, tmp_path: Path) -> None:
        archive = _targz()
        wrong = "0" * 64
        sums = f"{wrong}  {self.ASSET}\n".encode()

        with pytest.raises(CrwBinaryNotFoundError, match="checksum mismatch"):
            self._run(tmp_path, sums, archive)

        # Fails closed: nothing cached, and in particular no fallback to some
        # other binary that happens to be lying around.
        assert not list(tmp_path.rglob("crw-mcp"))

    def test_matching_digest_installs(self, tmp_path: Path) -> None:
        archive = _targz()
        good = hashlib.sha256(archive).hexdigest()
        sums = f"{good}  {self.ASSET}\n".encode()

        binary = self._run(tmp_path, sums, archive)
        assert binary.is_file()
        assert os.access(binary, os.X_OK)

    def test_binary_mode_marker_is_tolerated(self, tmp_path: Path) -> None:
        """coreutils writes "<hash> *<name>" for binary-mode entries."""
        archive = _targz()
        good = hashlib.sha256(archive).hexdigest()
        sums = f"{good} *{self.ASSET}\n".encode()

        assert self._run(tmp_path, sums, archive).is_file()

    def test_absent_sha256sums_raises(self, tmp_path: Path) -> None:
        """A release with no SHA256SUMS is unverifiable, so it is refused."""
        err = HTTPError("u", 404, "Not Found", {}, None)  # type: ignore[arg-type]
        with pytest.raises(CrwBinaryNotFoundError, match="no SHA256SUMS"):
            self._run(tmp_path, err, _targz())

    def test_unlisted_asset_raises(self, tmp_path: Path) -> None:
        sums = b"%s  some-other-asset.tar.gz\n" % (b"0" * 64)
        with pytest.raises(CrwBinaryNotFoundError, match="not listed"):
            self._run(tmp_path, sums, _targz())

    def test_network_error_is_distinguished_from_404(self, tmp_path: Path) -> None:
        with pytest.raises(CrwBinaryNotFoundError, match="could not reach GitHub"):
            self._run(tmp_path, URLError("no route to host"), _targz())


@pytest.mark.unit
class TestReleaseTag:
    def test_tag_follows_the_installed_version(self) -> None:
        with patch("crw._binary.importlib.metadata.version", return_value="0.27.0"):
            assert _release_tag() == "v0.27.0"

    def test_missing_distribution_names_the_escape_hatch(self) -> None:
        """Importable but not installed: fail with a usable next step."""
        with patch(
            "crw._binary.importlib.metadata.version",
            side_effect=importlib.metadata.PackageNotFoundError("crw"),
        ):
            with pytest.raises(CrwBinaryNotFoundError, match="CRW_BINARY"):
                _release_tag()


@pytest.mark.unit
class TestEnsureBinaryEndToEnd:
    """Drive the entrypoint, not the helper.

    `ensure_binary()` is what client.py and __main__.py actually call. A test
    that only exercises `_download_binary` leaves the resolution order itself
    unpinned, so a refactor could inline an unverified download and stay green.
    """

    ASSET = "crw-mcp-linux-x64.tar.gz"

    def _run(self, tmp_path: Path, sums: bytes, archive: bytes):
        responses: list[bytes] = [sums, archive]
        empty_path = tmp_path / "no-such-bin-dir"

        with (
            patch.dict(os.environ, {"PATH": str(empty_path)}, clear=True),
            patch("crw._binary.get_asset_name", return_value=self.ASSET),
            patch(
                "crw._binary.urlopen",
                side_effect=lambda *a, **k: io.BytesIO(responses.pop(0)),
            ),
            patch("crw._binary.user_cache_dir", return_value=str(tmp_path / "cache")),
            patch("crw._binary.importlib.metadata.version", return_value="9.9.9"),
        ):
            return ensure_binary()

    def test_mismatch_refused_through_the_entrypoint(self, tmp_path: Path) -> None:
        archive = _targz()
        sums = f"{'0' * 64}  {self.ASSET}\n".encode()

        with pytest.raises(CrwBinaryNotFoundError, match="checksum mismatch"):
            self._run(tmp_path, sums, archive)
        assert not list(tmp_path.rglob("crw-mcp"))

    def test_verified_archive_resolves_through_the_entrypoint(
        self, tmp_path: Path
    ) -> None:
        archive = _targz()
        sums = f"{hashlib.sha256(archive).hexdigest()}  {self.ASSET}\n".encode()

        binary = self._run(tmp_path, sums, archive)
        assert binary.is_file() and os.access(binary, os.X_OK)

    def test_empty_path_entry_does_not_exec_the_cwd(
        self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
    ) -> None:
        """A trailing colon in PATH means cwd; that must not resolve a binary."""
        planted = tmp_path / BINARY_NAME
        planted.write_bytes(b"\x7fELF" + b"\x00" * 64)
        planted.chmod(0o755)
        monkeypatch.chdir(tmp_path)

        archive = _targz()
        sums = f"{hashlib.sha256(archive).hexdigest()}  {self.ASSET}\n".encode()
        responses: list[bytes] = [sums, archive]

        with (
            patch.dict(os.environ, {"PATH": "/nonexistent:"}, clear=True),
            patch("crw._binary.get_asset_name", return_value=self.ASSET),
            patch(
                "crw._binary.urlopen",
                side_effect=lambda *a, **k: io.BytesIO(responses.pop(0)),
            ),
            patch("crw._binary.user_cache_dir", return_value=str(tmp_path / "cache")),
            patch("crw._binary.importlib.metadata.version", return_value="9.9.9"),
        ):
            resolved = ensure_binary()

        # Resolved from the verified download, never from the planted cwd file.
        assert resolved != planted
        assert "cache" in str(resolved)

    def test_non_executable_cache_entry_is_not_trusted(self, tmp_path: Path) -> None:
        """A partially-written cache entry must be re-downloaded, not returned.

        The npm launcher's staging comment cites this gate as the reason Python
        needs no staging of its own, so it is a cross-file invariant: assert it
        rather than leave it living in a comment.
        """
        cache_root = tmp_path / "cache"
        (cache_root / "v9.9.9").mkdir(parents=True)
        partial = cache_root / "v9.9.9" / BINARY_NAME
        partial.write_bytes(b"\x7fELF truncated")
        partial.chmod(0o644)

        archive = _targz()
        sums = f"{hashlib.sha256(archive).hexdigest()}  {self.ASSET}\n".encode()
        responses: list[bytes] = [sums, archive]

        with (
            patch.dict(os.environ, {"PATH": str(tmp_path / "nope")}, clear=True),
            patch("crw._binary.get_asset_name", return_value=self.ASSET),
            patch(
                "crw._binary.urlopen",
                side_effect=lambda *a, **k: io.BytesIO(responses.pop(0)),
            ),
            patch("crw._binary.user_cache_dir", return_value=str(cache_root)),
            patch("crw._binary.importlib.metadata.version", return_value="9.9.9"),
        ):
            resolved = ensure_binary()

        # Re-downloaded: the truncated placeholder was replaced, not returned.
        assert resolved.read_bytes() != b"\x7fELF truncated"
        assert os.access(resolved, os.X_OK)
