"""Unit tests for CrwClient — all network/subprocess calls are mocked."""

from __future__ import annotations

import json
from unittest.mock import MagicMock, patch

import pytest

from crw.client import CrwClient
from crw.exceptions import CrwApiError, CrwError, CrwTimeoutError


# ---------------------------------------------------------------------------
# Init mode detection
# ---------------------------------------------------------------------------


@pytest.mark.unit
class TestInit:
    def test_init_subprocess_mode(self) -> None:
        client = CrwClient()
        assert client._api_url is None
        assert client._api_key is None

    def test_init_http_mode(self) -> None:
        client = CrwClient(api_url="https://fastcrw.com/api", api_key="fc-test")
        assert client._api_url == "https://fastcrw.com/api"
        assert client._api_key == "fc-test"


# ---------------------------------------------------------------------------
# Scrape
# ---------------------------------------------------------------------------


@pytest.mark.unit
class TestScrape:
    def test_scrape_http_builds_correct_request(self) -> None:
        client = CrwClient(api_url="https://fastcrw.com/api", api_key="fc-test")
        mock_response = {"markdown": "# Hello", "metadata": {"title": "Hello"}}

        with patch.object(client, "_http_post", return_value=mock_response) as mock_post:
            result = client.scrape("https://example.com", formats=["markdown", "html"])

        mock_post.assert_called_once()
        call_args = mock_post.call_args
        assert call_args[0][0] == "/v1/scrape"
        body = call_args[0][1]
        assert body["url"] == "https://example.com"
        assert body["formats"] == ["markdown", "html"]
        assert result == mock_response

    def test_scrape_subprocess_calls_tool(self) -> None:
        client = CrwClient()
        mock_response = {"markdown": "# Hello"}

        with patch.object(client, "_tool_call", return_value=mock_response) as mock_tool:
            result = client.scrape("https://example.com")

        mock_tool.assert_called_once()
        call_args = mock_tool.call_args
        assert call_args[0][0] == "crw_scrape"
        assert call_args[0][1]["url"] == "https://example.com"
        assert result == mock_response


# ---------------------------------------------------------------------------
# Crawl
# ---------------------------------------------------------------------------


@pytest.mark.unit
class TestCrawl:
    def test_crawl_http_polls_until_complete(self) -> None:
        client = CrwClient(api_url="https://fastcrw.com/api", api_key="fc-test")

        crawl_start_response = {"id": "job-123"}
        poll_in_progress = {"success": True, "status": "scraping", "data": []}
        poll_completed = {
            "success": True,
            "status": "completed",
            "data": [{"url": "https://example.com", "markdown": "# Hello"}],
        }

        with (
            patch.object(client, "_http_post", return_value=crawl_start_response) as mock_post,
            patch.object(
                client, "_http_request", side_effect=[poll_in_progress, poll_completed]
            ) as mock_req,
            patch("crw.client.time.sleep"),
        ):
            result = client.crawl("https://example.com", poll_interval=0.01, timeout=10)

        assert mock_req.call_count == 2
        assert len(result) == 1
        assert result[0]["url"] == "https://example.com"

    def test_crawl_raises_timeout(self) -> None:
        client = CrwClient(api_url="https://fastcrw.com/api", api_key="fc-test")

        crawl_start_response = {"id": "job-timeout"}
        poll_in_progress = {"success": True, "status": "scraping", "data": []}

        with (
            patch.object(client, "_http_post", return_value=crawl_start_response),
            patch.object(client, "_http_request", return_value=poll_in_progress),
            patch("crw.client.time.sleep"),
            patch("crw.client.time.monotonic", side_effect=[0.0, 0.0, 100.0]),
        ):
            with pytest.raises(CrwTimeoutError, match="timed out"):
                client.crawl("https://example.com", poll_interval=0.01, timeout=5)

    def test_crawl_raises_on_failure(self) -> None:
        client = CrwClient(api_url="https://fastcrw.com/api", api_key="fc-test")

        crawl_start_response = {"id": "job-fail"}
        poll_failed = {"success": True, "status": "failed", "error": "Something went wrong"}

        with (
            patch.object(client, "_http_post", return_value=crawl_start_response),
            patch.object(client, "_http_request", return_value=poll_failed),
            patch("crw.client.time.sleep"),
        ):
            with pytest.raises(CrwError, match="Crawl failed"):
                client.crawl("https://example.com", poll_interval=0.01, timeout=10)


# ---------------------------------------------------------------------------
# Map
# ---------------------------------------------------------------------------


@pytest.mark.unit
class TestMap:
    def test_map_http_returns_links(self) -> None:
        client = CrwClient(api_url="https://fastcrw.com/api", api_key="fc-test")
        mock_response = {"links": ["https://example.com/a", "https://example.com/b"]}

        with patch.object(client, "_http_post", return_value=mock_response):
            result = client.map("https://example.com")

        assert result == ["https://example.com/a", "https://example.com/b"]


# ---------------------------------------------------------------------------
# Search
# ---------------------------------------------------------------------------


@pytest.mark.unit
class TestSearch:
    def test_search_requires_api_url(self) -> None:
        client = CrwClient()
        with pytest.raises(CrwError, match="requires api_url"):
            client.search("python web scraping")

    def test_search_http_sends_query(self) -> None:
        client = CrwClient(api_url="https://fastcrw.com/api", api_key="fc-test")
        mock_response = [{"url": "https://example.com", "title": "Example"}]

        with patch.object(client, "_http_post", return_value=mock_response) as mock_post:
            result = client.search("python web scraping", limit=5)

        mock_post.assert_called_once()
        call_args = mock_post.call_args
        assert call_args[0][0] == "/v1/search"
        body = call_args[0][1]
        assert body["query"] == "python web scraping"
        assert body["limit"] == 5
        assert result == mock_response


# ---------------------------------------------------------------------------
# Close / context manager
# ---------------------------------------------------------------------------


@pytest.mark.unit
class TestLifecycle:
    def test_close_terminates_process(self) -> None:
        client = CrwClient()
        mock_proc = MagicMock()
        mock_proc.poll.return_value = None  # process is alive
        mock_proc.wait.return_value = 0
        client._process = mock_proc

        client.close()

        mock_proc.stdin.close.assert_called_once()
        assert client._process is None

    def test_context_manager(self) -> None:
        client = CrwClient(api_url="https://fastcrw.com/api")
        with patch.object(client, "close") as mock_close:
            with client as c:
                assert c is client
            mock_close.assert_called_once()


# ---------------------------------------------------------------------------
# Error handling
# ---------------------------------------------------------------------------


@pytest.mark.unit
class TestErrors:
    def test_api_error_raised_on_failure(self) -> None:
        client = CrwClient(api_url="https://fastcrw.com/api", api_key="fc-test")

        with patch.object(
            client,
            "_http_request",
            side_effect=CrwApiError("Bad request", status_code=400),
        ):
            with pytest.raises(CrwApiError) as exc_info:
                client.scrape("https://example.com")
            assert exc_info.value.status_code == 400

    def test_jsonrpc_error_handling(self) -> None:
        client = CrwClient()
        mock_proc = MagicMock()
        error_response = json.dumps({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {"code": -32600, "message": "Invalid request"},
        })
        mock_proc.stdout.readline.return_value = error_response + "\n"
        mock_proc.poll.return_value = None
        client._process = mock_proc

        with pytest.raises(CrwApiError, match="Invalid request"):
            client._jsonrpc("tools/call", {"name": "crw_scrape", "arguments": {}})
