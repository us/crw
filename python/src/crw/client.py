"""CRW client — subprocess (embedded) or HTTP mode."""

from __future__ import annotations

import json
import subprocess
import time
from typing import Any

from crw._binary import ensure_binary
from crw.exceptions import CrwApiError, CrwError, CrwTimeoutError

_REQUEST_ID = 0


def _next_id() -> int:
    global _REQUEST_ID
    _REQUEST_ID += 1
    return _REQUEST_ID


class CrwClient:
    """CRW web scraper client.

    Args:
        api_url: CRW server URL for HTTP mode. If None, uses subprocess mode
                 (spawns crw-mcp binary, no server required).
        api_key: API key for authentication (HTTP mode or fastcrw.com).

    Examples:
        # Subprocess mode (zero config, no server):
        client = CrwClient()
        result = client.scrape("https://example.com")

        # HTTP mode (remote server):
        client = CrwClient(api_url="https://fastcrw.com/api", api_key="fc-...")
        result = client.scrape("https://example.com")
    """

    def __init__(self, api_url: str | None = None, api_key: str | None = None):
        self._api_url = api_url
        self._api_key = api_key
        self._process: subprocess.Popen | None = None

    def scrape(
        self,
        url: str,
        formats: list[str] | None = None,
        only_main_content: bool = True,
        include_tags: list[str] | None = None,
        exclude_tags: list[str] | None = None,
        **kwargs: Any,
    ) -> dict:
        """Scrape a single URL and return its content.

        Returns:
            Dict with keys like 'markdown', 'html', 'metadata', etc.
        """
        args: dict[str, Any] = {"url": url, "onlyMainContent": only_main_content}
        if formats:
            args["formats"] = formats
        if include_tags:
            args["includeTags"] = include_tags
        if exclude_tags:
            args["excludeTags"] = exclude_tags
        args.update(kwargs)

        if self._api_url:
            return self._http_post("/v1/scrape", args)
        return self._tool_call("crw_scrape", args)

    def crawl(
        self,
        url: str,
        max_depth: int = 2,
        max_pages: int = 10,
        poll_interval: float = 2.0,
        timeout: float = 300.0,
        **kwargs: Any,
    ) -> list[dict]:
        """Crawl a website and return all page results.

        Starts an async crawl, polls for completion, and returns all results.
        """
        args: dict[str, Any] = {"url": url, "maxDepth": max_depth, "maxPages": max_pages}
        args.update(kwargs)

        if self._api_url:
            return self._http_crawl(args, poll_interval, timeout)

        # Subprocess mode: start crawl, poll status
        result = self._tool_call("crw_crawl", args)
        job_id = result.get("id")
        if not job_id:
            raise CrwError(f"Crawl did not return job ID: {result}")

        return self._poll_crawl(job_id, poll_interval, timeout)

    def map(
        self,
        url: str,
        max_depth: int = 2,
        use_sitemap: bool = True,
        **kwargs: Any,
    ) -> list[str]:
        """Discover URLs on a website.

        Returns:
            List of discovered URLs.
        """
        args: dict[str, Any] = {"url": url, "maxDepth": max_depth, "useSitemap": use_sitemap}
        args.update(kwargs)

        if self._api_url:
            data = self._http_post("/v1/map", args)
            return data.get("links", [])

        result = self._tool_call("crw_map", args)
        return result.get("links", [])

    def close(self) -> None:
        """Shut down the subprocess if running."""
        if self._process and self._process.poll() is None:
            self._process.stdin.close()
            try:
                self._process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self._process.terminate()
                self._process.wait(timeout=5)
            self._process = None

    def __enter__(self) -> CrwClient:
        return self

    def __exit__(self, *_: Any) -> None:
        self.close()

    def __del__(self) -> None:
        self.close()

    # --- Subprocess (embedded) mode ---

    def _ensure_process(self) -> subprocess.Popen:
        if self._process is None or self._process.poll() is not None:
            binary = ensure_binary()
            self._process = subprocess.Popen(
                [str(binary)],
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                text=True,
            )
        return self._process

    def _jsonrpc(self, method: str, params: dict | None = None) -> Any:
        proc = self._ensure_process()
        req = {
            "jsonrpc": "2.0",
            "id": _next_id(),
            "method": method,
            "params": params or {},
        }
        proc.stdin.write(json.dumps(req) + "\n")
        proc.stdin.flush()

        line = proc.stdout.readline()
        if not line:
            raise CrwError("crw-mcp process closed unexpectedly")

        resp = json.loads(line)
        if "error" in resp:
            raise CrwApiError(resp["error"].get("message", str(resp["error"])))
        return resp.get("result")

    def _tool_call(self, tool_name: str, arguments: dict) -> dict:
        result = self._jsonrpc("tools/call", {"name": tool_name, "arguments": arguments})
        if not result or not result.get("content"):
            raise CrwError(f"Empty response from {tool_name}")

        content = result["content"][0]
        if result.get("isError"):
            raise CrwApiError(content.get("text", "Unknown error"))

        return json.loads(content["text"])

    def _poll_crawl(self, job_id: str, poll_interval: float, timeout: float) -> list[dict]:
        start = time.monotonic()
        while True:
            if time.monotonic() - start > timeout:
                raise CrwTimeoutError(f"Crawl {job_id} timed out after {timeout}s")

            result = self._tool_call("crw_check_crawl_status", {"id": job_id})
            status = result.get("status")

            if status == "completed":
                return result.get("data", [])
            if status == "failed":
                raise CrwError(f"Crawl failed: {result.get('error', 'unknown')}")

            time.sleep(poll_interval)

    # --- HTTP mode ---

    def _http_request(self, method: str, path: str, body: dict | None = None) -> dict:
        import urllib.request

        url = f"{self._api_url.rstrip('/')}{path}"
        headers = {"Content-Type": "application/json"}
        if self._api_key:
            headers["Authorization"] = f"Bearer {self._api_key}"

        data = json.dumps(body).encode() if body else None
        req = urllib.request.Request(url, data=data, headers=headers, method=method)

        with urllib.request.urlopen(req, timeout=120) as resp:
            result = json.loads(resp.read())

        if not result.get("success"):
            raise CrwApiError(result.get("error", "API error"))
        return result.get("data", result)

    def _http_post(self, path: str, body: dict) -> dict:
        return self._http_request("POST", path, body)

    def _http_get(self, path: str) -> dict:
        return self._http_request("GET", path)

    def _http_crawl(self, args: dict, poll_interval: float, timeout: float) -> list[dict]:
        result = self._http_post("/v1/crawl", args)
        job_id = result.get("id")
        if not job_id:
            raise CrwError(f"Crawl did not return job ID: {result}")

        start = time.monotonic()
        while True:
            if time.monotonic() - start > timeout:
                raise CrwTimeoutError(f"Crawl {job_id} timed out after {timeout}s")

            status_result = self._http_get(f"/v1/crawl/{job_id}")
            status = status_result.get("status")

            if status == "completed":
                return status_result.get("data", [])
            if status == "failed":
                raise CrwError(f"Crawl failed: {status_result.get('error', 'unknown')}")

            time.sleep(poll_interval)
