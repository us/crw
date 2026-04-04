"""Integration tests for CrwClient — hit real fastcrw.com API.

These tests require CRW_API_KEY to be set in the environment.
They are automatically skipped when the key is not available.
"""

from __future__ import annotations

import pytest

from crw import CrwClient


@pytest.mark.integration
@pytest.mark.timeout(30)
class TestScrapeIntegration:
    def test_scrape_real_url(self, cloud_client: CrwClient) -> None:
        result = cloud_client.scrape("https://example.com")
        assert "markdown" in result
        assert len(result["markdown"]) > 0
        assert "metadata" in result
        assert result["metadata"].get("title")

    def test_scrape_with_formats(self, cloud_client: CrwClient) -> None:
        result = cloud_client.scrape(
            "https://example.com", formats=["markdown", "html"]
        )
        assert "markdown" in result
        assert "html" in result

    def test_scrape_invalid_url(self, cloud_client: CrwClient) -> None:
        """Scraping a non-URL should raise an error or return empty content."""
        with pytest.raises(Exception):
            cloud_client.scrape("not-a-url")


@pytest.mark.integration
@pytest.mark.timeout(60)
class TestCrawlIntegration:
    def test_crawl_real_site(self, cloud_client: CrwClient) -> None:
        results = cloud_client.crawl(
            "https://example.com", max_pages=2, timeout=60
        )
        assert isinstance(results, list)
        assert len(results) >= 1


@pytest.mark.integration
@pytest.mark.timeout(30)
class TestMapIntegration:
    def test_map_real_site(self, cloud_client: CrwClient) -> None:
        links = cloud_client.map("https://example.com")
        assert isinstance(links, list)
        assert len(links) >= 1
        assert all(isinstance(link, str) for link in links)


@pytest.mark.integration
@pytest.mark.timeout(30)
class TestSearchIntegration:
    def test_search_web(self, cloud_client: CrwClient) -> None:
        results = cloud_client.search("python web scraping")
        assert isinstance(results, (list, dict))
        if isinstance(results, list):
            assert len(results) > 0
            first = results[0]
            assert "url" in first
            assert "title" in first

    def test_search_with_limit(self, cloud_client: CrwClient) -> None:
        results = cloud_client.search("python web scraping", limit=3)
        if isinstance(results, list):
            assert len(results) <= 3

    def test_search_with_scrape_options(self, cloud_client: CrwClient) -> None:
        results = cloud_client.search(
            "python web scraping",
            limit=2,
            scrape_options={"formats": ["markdown"]},
        )
        if isinstance(results, list) and len(results) > 0:
            # At least one result should have markdown content
            has_markdown = any(r.get("markdown") for r in results)
            assert has_markdown


@pytest.mark.integration
@pytest.mark.timeout(30)
class TestContextManagerIntegration:
    def test_client_context_manager(self, api_key: str) -> None:
        with CrwClient(
            api_url="https://fastcrw.com/api", api_key=api_key
        ) as client:
            result = client.scrape("https://example.com")
            assert "markdown" in result
            assert len(result["markdown"]) > 0
