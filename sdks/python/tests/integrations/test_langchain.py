"""Tests for CrwLoader with mocked CrwClient."""

import os
from typing import Iterator
from unittest.mock import MagicMock, patch

import pytest
from langchain_core.documents import Document

from crw.integrations.langchain import CrwLoader


@pytest.fixture
def mock_client():
    with patch("crw.integrations.langchain.CrwClient") as mock_cls:
        client = MagicMock()
        mock_cls.return_value = client
        yield client


class TestInit:
    def test_default_api_url(self, mock_client):
        loader = CrwLoader(url="https://example.com")
        assert loader.api_url is None

    def test_custom_api_url(self, mock_client):
        loader = CrwLoader(url="https://example.com", api_url="https://fastcrw.com/api")
        assert loader.api_url == "https://fastcrw.com/api"

    def test_api_key_stored(self, mock_client):
        loader = CrwLoader(url="https://example.com", api_key="test-key")
        assert loader.api_key == "test-key"

    @patch.dict(os.environ, {}, clear=True)
    def test_no_api_key_is_none(self, mock_client):
        loader = CrwLoader(url="https://example.com")
        assert loader.api_key is None

    @patch.dict(os.environ, {"CRW_API_KEY": "env-key"})
    def test_api_key_from_env(self, mock_client):
        loader = CrwLoader(url="https://example.com")
        assert loader.api_key == "env-key"

    @patch.dict(os.environ, {"CRW_API_URL": "https://env-url.com"})
    def test_api_url_from_env(self, mock_client):
        loader = CrwLoader(url="https://example.com")
        assert loader.api_url == "https://env-url.com"

    def test_search_mode_requires_query(self, mock_client):
        with pytest.raises(ValueError, match="query is required"):
            CrwLoader(url="", mode="search")

    def test_non_search_mode_requires_url(self, mock_client):
        with pytest.raises(ValueError, match="url is required"):
            CrwLoader(url="", mode="scrape")


class TestScrape:
    def test_scrape_returns_document(self, mock_client):
        mock_client.scrape.return_value = {
            "markdown": "# Hello World",
            "metadata": {"title": "Hello", "sourceURL": "https://example.com"},
        }

        loader = CrwLoader(url="https://example.com", mode="scrape")
        docs = list(loader.lazy_load())

        assert len(docs) == 1
        assert docs[0].page_content == "# Hello World"
        assert docs[0].metadata["title"] == "Hello"

    def test_scrape_with_params(self, mock_client):
        mock_client.scrape.return_value = {"markdown": "ok", "metadata": {}}

        loader = CrwLoader(
            url="https://example.com",
            mode="scrape",
            params={"render_js": True, "css_selector": "article"},
        )
        list(loader.lazy_load())

        call_kwargs = mock_client.scrape.call_args[1]
        assert call_kwargs["render_js"] is True
        assert call_kwargs["css_selector"] == "article"

    def test_scrape_empty_data_yields_nothing(self, mock_client):
        mock_client.scrape.return_value = {}

        loader = CrwLoader(url="https://example.com", mode="scrape")
        assert list(loader.lazy_load()) == []

    def test_scrape_fallback_to_html(self, mock_client):
        mock_client.scrape.return_value = {
            "html": "<h1>Hi</h1>",
            "metadata": {},
        }

        loader = CrwLoader(url="https://example.com", mode="scrape")
        docs = list(loader.lazy_load())
        assert docs[0].page_content == "<h1>Hi</h1>"

    def test_scrape_empty_string_content_yields_nothing(self, mock_client):
        mock_client.scrape.return_value = {"markdown": "", "metadata": {}}

        loader = CrwLoader(url="https://example.com", mode="scrape")
        docs = list(loader.lazy_load())
        assert docs == []


class TestCrawl:
    def test_crawl_returns_documents(self, mock_client):
        mock_client.crawl.return_value = [
            {"markdown": "# Page 1", "metadata": {"sourceURL": "https://example.com"}},
            {"markdown": "# Page 2", "metadata": {"sourceURL": "https://example.com/about"}},
        ]

        loader = CrwLoader(url="https://example.com", mode="crawl")
        docs = list(loader.lazy_load())

        assert len(docs) == 2
        assert docs[0].page_content == "# Page 1"
        assert docs[1].metadata["sourceURL"] == "https://example.com/about"

    def test_crawl_passes_poll_params(self, mock_client):
        mock_client.crawl.return_value = [
            {"markdown": "# Done", "metadata": {}},
        ]

        loader = CrwLoader(
            url="https://example.com",
            mode="crawl",
            params={"poll_interval": 5, "timeout": 60},
        )
        list(loader.lazy_load())

        call_kwargs = mock_client.crawl.call_args
        assert call_kwargs[1]["poll_interval"] == 5
        assert call_kwargs[1]["timeout"] == 60

    def test_poll_interval_clamped_to_minimum(self, mock_client):
        mock_client.crawl.return_value = [
            {"markdown": "# Done", "metadata": {}},
        ]

        loader = CrwLoader(
            url="https://example.com",
            mode="crawl",
            params={"poll_interval": 0.0},
        )
        list(loader.lazy_load())

        call_kwargs = mock_client.crawl.call_args
        assert call_kwargs[1]["poll_interval"] == 0.1

    def test_crawl_failed(self, mock_client):
        from crw.exceptions import CrwError

        mock_client.crawl.side_effect = CrwError("Crawl failed: unknown")

        loader = CrwLoader(url="https://example.com", mode="crawl")
        with pytest.raises(CrwError, match="failed"):
            list(loader.lazy_load())

    def test_crawl_timeout(self, mock_client):
        from crw.exceptions import CrwTimeoutError

        mock_client.crawl.side_effect = CrwTimeoutError("Crawl timed out")

        loader = CrwLoader(
            url="https://example.com",
            mode="crawl",
            params={"timeout": 4, "poll_interval": 2},
        )
        with pytest.raises(CrwTimeoutError):
            list(loader.lazy_load())


class TestMap:
    def test_map_returns_urls(self, mock_client):
        mock_client.map.return_value = [
            "https://example.com",
            "https://example.com/about",
        ]

        loader = CrwLoader(url="https://example.com", mode="map")
        docs = list(loader.lazy_load())

        assert len(docs) == 2
        assert docs[0].page_content == "https://example.com"

    def test_map_skips_empty(self, mock_client):
        mock_client.map.return_value = ["https://example.com", ""]

        loader = CrwLoader(url="https://example.com", mode="map")
        assert len(list(loader.lazy_load())) == 1


class TestSearch:
    def test_search_returns_documents(self, mock_client):
        mock_client.search.return_value = [
            {"title": "Result 1", "url": "https://example.com", "description": "First"},
        ]

        loader = CrwLoader(
            url="",
            mode="search",
            query="test query",
            api_url="https://fastcrw.com/api",
        )
        docs = list(loader.lazy_load())

        assert len(docs) == 1
        assert docs[0].metadata["title"] == "Result 1"
        assert docs[0].metadata["source"] == "search"
        assert docs[0].page_content == "First"

    def test_search_grouped_results(self, mock_client):
        mock_client.search.return_value = {
            "web": [
                {"title": "Web Result", "url": "https://web.com", "description": "Web desc"},
            ],
            "news": [
                {"title": "News Result", "url": "https://news.com", "description": "News desc"},
                {"title": "News 2", "url": "https://news2.com", "markdown": "# Breaking"},
            ],
        }

        loader = CrwLoader(
            url="",
            mode="search",
            query="grouped query",
            api_url="https://fastcrw.com/api",
        )
        docs = list(loader.lazy_load())

        assert len(docs) == 3
        assert docs[0].metadata["source_type"] == "web"
        assert docs[0].metadata["source"] == "search"
        assert docs[0].page_content == "Web desc"
        assert docs[1].metadata["source_type"] == "news"
        assert docs[1].page_content == "News desc"
        assert docs[2].page_content == "# Breaking"


class TestInterface:
    def test_lazy_load_returns_iterator(self, mock_client):
        mock_client.scrape.return_value = {"markdown": "# Test", "metadata": {}}

        loader = CrwLoader(url="https://example.com")
        assert isinstance(loader.lazy_load(), Iterator)

    def test_load_returns_list(self, mock_client):
        mock_client.scrape.return_value = {"markdown": "# Test", "metadata": {}}

        loader = CrwLoader(url="https://example.com")
        result = loader.load()
        assert isinstance(result, list)

    def test_invalid_mode(self, mock_client):
        loader = CrwLoader(url="https://example.com")
        loader.mode = "invalid"
        with pytest.raises(ValueError, match="Invalid mode"):
            list(loader.lazy_load())

    def test_close_cleans_up(self, mock_client):
        loader = CrwLoader(url="https://example.com")
        # Force client creation
        loader._get_client()
        loader.close()
        assert loader._client is None
        mock_client.close.assert_called_once()


class TestParseMode:
    def test_parse_loads_file(self, mock_client):
        mock_client.parse_file.return_value = {
            "markdown": "# Doc",
            "metadata": {"numPages": 2},
        }
        loader = CrwLoader(url="/tmp/doc.pdf", mode="parse", params={"formats": ["markdown"]})
        docs = loader.load()
        assert len(docs) == 1
        assert docs[0].page_content == "# Doc"
        assert docs[0].metadata["numPages"] == 2
        # path forwarded, plus only parse-relevant params
        assert mock_client.parse_file.call_args[0][0] == "/tmp/doc.pdf"
        assert mock_client.parse_file.call_args[1]["formats"] == ["markdown"]

    def test_parse_requires_path(self, mock_client):
        with pytest.raises(ValueError, match="url is required"):
            CrwLoader(url="", mode="parse")


class TestExtractMode:
    def test_extract_yields_json_document(self, mock_client):
        mock_client.extract.return_value = {"title": "Hello"}
        loader = CrwLoader(
            url="https://example.com",
            mode="extract",
            api_url="https://fastcrw.com/api",
            query="Get the title",
            params={"schema": {"type": "object"}},
        )
        docs = loader.load()
        assert len(docs) == 1
        assert '"title": "Hello"' in docs[0].page_content
        assert docs[0].metadata["source"] == "extract"
        # prompt comes from query, schema from params
        assert mock_client.extract.call_args[1]["prompt"] == "Get the title"
        assert mock_client.extract.call_args[1]["schema"] == {"type": "object"}
