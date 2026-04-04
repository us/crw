"""Shared fixtures and markers for CRW SDK tests."""

from __future__ import annotations

import os

import pytest

from crw import CrwClient


def pytest_configure(config: pytest.Config) -> None:
    config.addinivalue_line("markers", "unit: pure unit tests (no network)")
    config.addinivalue_line(
        "markers", "integration: tests that hit real APIs (need CRW_API_KEY env var)"
    )


def pytest_collection_modifyitems(items: list[pytest.Item]) -> None:
    """Auto-skip integration tests when CRW_API_KEY is not set."""
    skip_integration = pytest.mark.skip(reason="CRW_API_KEY env var not set")
    for item in items:
        if "integration" in item.keywords and not os.environ.get("CRW_API_KEY"):
            item.add_marker(skip_integration)


@pytest.fixture
def api_key() -> str:
    """Return CRW_API_KEY from the environment."""
    key = os.environ.get("CRW_API_KEY", "")
    return key


@pytest.fixture
def cloud_client(api_key: str) -> CrwClient:
    """Create a CrwClient pointing at the cloud API."""
    return CrwClient(api_url="https://fastcrw.com/api", api_key=api_key)
