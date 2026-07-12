"""CRW client — subprocess (embedded) or HTTP mode."""

from __future__ import annotations

import base64
import json
import os
import subprocess
import time
from typing import Any
from urllib.parse import quote, urlencode

from crw._binary import ensure_binary
from crw.exceptions import CrwApiError, CrwError, CrwTimeoutError

_REQUEST_ID = 0

# CRW is cloud-first: with no explicit api_url and no CRW_LOCAL opt-in, the client
# talks to the managed cloud. These mirror the CLI onboarding (setup/cloud.rs).
CLOUD_API_URL = "https://api.fastcrw.com"
DASHBOARD_URL = "https://fastcrw.com/dashboard"
DOCS_URL = "https://us.github.io/crw"

_SIGNUP_NUDGE = (
    "No CRW API key found. CRW uses the managed cloud ({cloud}) by default.\n"
    "  → Sign up at {dashboard} for 500 free credits — no payment, no monthly "
    "reset (GitHub/Google, ~10s) — then set CRW_API_KEY (or pass api_key=...).\n"
    "  → Prefer to self-host? Set CRW_LOCAL=1 to run the local engine. Docs: {docs}"
).format(cloud=CLOUD_API_URL, dashboard=DASHBOARD_URL, docs=DOCS_URL)

# Endpoints that have no MCP tool and therefore only work in HTTP mode
# (a running crw-server / cloud), mirroring how search() needs SearXNG.
_HTTP_ONLY_HINT = (
    "{name}() requires HTTP mode ({reason}). It is not available with CRW_LOCAL=1. "
    "Use the cloud (set CRW_API_KEY) or pass api_url=... for a self-hosted server."
)


def _env_truthy(value: str | None) -> bool:
    return bool(value) and value.strip().lower() not in {"0", "false", "no", ""}


def _next_id() -> int:
    global _REQUEST_ID
    _REQUEST_ID += 1
    return _REQUEST_ID


def _read_json_response(req: Any) -> dict:
    """Send a urllib request and parse the JSON body.

    On a non-2xx status, urllib raises ``HTTPError``; we read its JSON body and
    surface the engine's ``error`` message as a ``CrwApiError`` instead of a bare
    ``HTTPError`` so callers get a useful message.
    """
    import urllib.error
    import urllib.request

    try:
        with urllib.request.urlopen(req, timeout=120) as resp:
            return json.loads(resp.read())
    except urllib.error.HTTPError as e:
        try:
            payload = json.loads(e.read())
        except Exception:
            raise CrwApiError(f"HTTP {e.code}: {e.reason}") from e
        message = payload.get("error") or payload.get("message") or f"HTTP {e.code}"
        raise CrwApiError(message) from e


def _encode_multipart(filename: str, content: bytes, options: dict[str, Any]) -> tuple[bytes, str]:
    """Encode a ``file`` part + JSON ``options`` part as multipart/form-data.

    Matches the ``/v2/parse`` handler, which reads exactly these two field names.
    """
    boundary = "----crwFormBoundary7MA4YWxkTrZu0gW"
    crlf = b"\r\n"
    buf = bytearray()

    buf += b"--" + boundary.encode() + crlf
    buf += (
        f'Content-Disposition: form-data; name="file"; filename="{filename}"'.encode() + crlf
    )
    buf += b"Content-Type: application/octet-stream" + crlf + crlf
    buf += content + crlf

    if options:
        buf += b"--" + boundary.encode() + crlf
        buf += b'Content-Disposition: form-data; name="options"' + crlf
        buf += b"Content-Type: application/json" + crlf + crlf
        buf += json.dumps(options).encode() + crlf

    buf += b"--" + boundary.encode() + b"--" + crlf
    return bytes(buf), f"multipart/form-data; boundary={boundary}"


class CrwClient:
    """CRW web scraper client.

    CRW is cloud-first. With no arguments the client talks to the managed cloud
    (``api.fastcrw.com``) and needs an API key — sign up for 500 free credits at
    https://fastcrw.com/dashboard. To self-host the engine locally instead, set
    ``CRW_LOCAL=1`` (zero-config subprocess mode, no key, no server).

    Args:
        api_url: Explicit CRW server URL (e.g. your self-hosted ``crw-server``).
                 If omitted, defaults to the managed cloud unless ``CRW_LOCAL`` is
                 set. Also read from ``CRW_API_URL``.
        api_key: API key for the cloud (or an authenticated self-hosted server).
                 Also read from ``CRW_API_KEY``.

    Examples:
        # Cloud (default) — needs a key (CRW_API_KEY env or api_key=...):
        client = CrwClient()
        result = client.scrape("https://example.com")

        # Self-hosted server:
        client = CrwClient(api_url="http://localhost:3000")

        # Local zero-config engine (no server, no key) — set CRW_LOCAL=1.

    Feature availability:
        * scrape / crawl / map / parse_file — work in both local and HTTP mode.
        * search — works in both modes, but local mode needs a search backend
          configured ([search].searxng_url); cloud has it preconfigured.
        * extract / batch_scrape / capabilities / change_tracking_diff — HTTP mode
          only. The engine's MCP server does expose crw_extract, but this SDK's
          local transport does not route it yet.
    """

    def __init__(self, api_url: str | None = None, api_key: str | None = None):
        self._api_key = api_key or os.environ.get("CRW_API_KEY")
        self._process: subprocess.Popen | None = None

        if _env_truthy(os.environ.get("CRW_LOCAL")):
            # Explicit self-host opt-in: zero-config local engine (subprocess),
            # no cloud, no key required.
            self._api_url = None
            return

        explicit_url = api_url or os.environ.get("CRW_API_URL")
        # Cloud-first default: point at the managed cloud when no server is given.
        self._api_url = explicit_url or CLOUD_API_URL
        # Only the managed-cloud default requires a key; an explicit self-hosted
        # server may run without auth.
        if explicit_url is None and not self._api_key:
            raise CrwError(_SIGNUP_NUDGE)

    def scrape(
        self,
        url: str,
        formats: list[str] | None = None,
        only_main_content: bool = True,
        include_tags: list[str] | None = None,
        exclude_tags: list[str] | None = None,
        *,
        render_js: bool | None = None,
        renderer: str | None = None,
        wait_for: int | None = None,
        json_schema: dict | None = None,
        **kwargs: Any,
    ) -> dict:
        """Scrape a single URL and return its content.

        Args:
            url: The URL to scrape.
            formats: Output formats, e.g. ``["markdown", "html", "json"]``.
            only_main_content: Strip nav/boilerplate (default True).
            include_tags / exclude_tags: HTML tag allow/deny lists.
            render_js: Force the JS renderer on/off (engine ``renderJs``).
            renderer: Pin a specific renderer tier (engine ``renderer``).
            wait_for: Milliseconds to wait after load before extracting (``waitFor``).
            json_schema: A JSON Schema for structured LLM extraction. Automatically
                adds the ``json`` format. Requires an LLM provider configured on the
                engine.
            **kwargs: Any other engine scrape option, passed through verbatim.

        Returns:
            Dict with keys like 'markdown', 'html', 'json', 'metadata', etc.
        """
        args: dict[str, Any] = {"url": url, "onlyMainContent": only_main_content}
        if formats:
            args["formats"] = list(formats)
        if include_tags:
            args["includeTags"] = include_tags
        if exclude_tags:
            args["excludeTags"] = exclude_tags
        if render_js is not None:
            args["renderJs"] = render_js
        if renderer is not None:
            args["renderer"] = renderer
        if wait_for is not None:
            args["waitFor"] = wait_for
        if json_schema is not None:
            args["jsonSchema"] = json_schema
            fmts = args.get("formats") or []
            if "json" not in fmts:
                args["formats"] = [*fmts, "json"]
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

    def search(
        self,
        query: str,
        limit: int = 5,
        lang: str | None = None,
        tbs: str | None = None,
        sources: list[str] | None = None,
        categories: list[str] | None = None,
        scrape_options: dict[str, Any] | None = None,
        **kwargs: Any,
    ) -> list[dict] | dict:
        """Search the web and optionally scrape results.

        Works in both modes. In subprocess mode the engine needs a search backend
        configured (``[search].searxng_url`` or ``CRW_SEARCH__SEARXNG_URL``); the
        managed cloud has one preconfigured. If search is not configured the engine
        returns a clear ``search_disabled`` error.

        Args:
            query: Search query string.
            limit: Maximum number of results (1-20, default 5).
            lang: Language code for results (e.g. ``"en"``, ``"tr"``).
            tbs: Time filter (``"qdr:h"``, ``"qdr:d"``, ``"qdr:w"``, ``"qdr:m"``, ``"qdr:y"``).
            sources: Result types (``"web"``, ``"news"``, ``"images"``). Groups response when set.
            categories: Category filters (``"github"``, ``"research"``, ``"pdf"``).
            scrape_options: Scrape each result URL, e.g. ``{"formats": ["markdown"]}``.

        Returns:
            List of result dicts (flat) or dict grouped by source type.
        """
        args: dict[str, Any] = {"query": query, "limit": limit}
        if lang:
            args["lang"] = lang
        if tbs:
            args["tbs"] = tbs
        if sources:
            args["sources"] = sources
        if categories:
            args["categories"] = categories
        if scrape_options:
            args["scrapeOptions"] = scrape_options
        args.update(kwargs)

        if self._api_url:
            return self._http_post("/v1/search", args)
        return self._tool_call("crw_search", args)

    # --- Firecrawl-compatible Research API (cloud only) ---------------------

    def _research_get(self, path: str, params: dict) -> dict:
        if not self._api_url:
            raise CrwError("research API requires cloud mode (set api_key)")
        clean = {k: v for k, v in params.items() if v is not None}
        qs = urlencode(clean)
        return self._http_request("GET", f"{path}?{qs}" if qs else path, check_success=False)

    def search_papers(
        self,
        query: str,
        *,
        k: int | None = None,
        authors: str | None = None,
        categories: str | None = None,
        from_: str | None = None,
        to: str | None = None,
    ) -> dict:
        """Ranked paper search over `/v2/search/research/papers`."""
        return self._research_get(
            "/v2/search/research/papers",
            {
                "query": query,
                "k": k,
                "authors": authors,
                "categories": categories,
                "from": from_,
                "to": to,
            },
        )

    def get_paper(self, paper_id: str, *, query: str | None = None, k: int | None = None) -> dict:
        """Inspect metadata, or (with `query`) read top passages for a paper."""
        return self._research_get(
            f"/v2/search/research/papers/{quote(paper_id, safe='')}",
            {"query": query, "k": k},
        )

    def related_papers(
        self,
        paper_id: str,
        intent: str,
        *,
        mode: str | None = None,
        k: int | None = None,
    ) -> dict:
        """Citation-graph expansion (mode=similar|citers|references)."""
        return self._research_get(
            f"/v2/search/research/papers/{quote(paper_id, safe='')}/similar",
            {"intent": intent, "mode": mode, "k": k},
        )

    def search_github(self, query: str, *, k: int | None = None) -> dict:
        """GitHub history/README search over `/v2/search/research/github`."""
        return self._research_get("/v2/search/research/github", {"query": query, "k": k})

    def parse_file(
        self,
        path: str | None = None,
        *,
        content: bytes | None = None,
        filename: str | None = None,
        formats: list[str] | None = None,
        json_schema: dict | None = None,
        parsers: list[str | dict] | None = None,
        **kwargs: Any,
    ) -> dict:
        """Parse an uploaded document (PDF) into markdown / structured JSON.

        Works in both modes. Provide either ``path`` (a file on disk) or raw
        ``content`` bytes (with an optional ``filename``). Only PDF is supported
        by the engine today.

        Args:
            path: Path to a PDF file on disk.
            content: Raw file bytes (alternative to ``path``).
            filename: Name to report (defaults to the basename of ``path``).
            formats: Output formats, e.g. ``["markdown", "json"]``.
            json_schema: JSON Schema for structured LLM extraction from the document.
            parsers: Explicit parser selection.  Accepts bare strings
                (``["pdf"]``) or parser-spec dicts
                (``[{"type": "pdf", "maxPages": 10}]``).

        Returns:
            Dict with keys like 'markdown', 'json', 'metadata' (numPages, …).
        """
        if content is None:
            if path is None:
                raise CrwError("parse_file requires either path= or content=")
            with open(path, "rb") as f:
                content = f.read()
            if filename is None:
                filename = os.path.basename(path)
        if filename is None:
            filename = "document.pdf"

        if self._api_url:
            options: dict[str, Any] = {}
            if formats:
                options["formats"] = list(formats)
            if json_schema is not None:
                options["jsonSchema"] = json_schema
            if parsers:
                options["parsers"] = parsers
            options.update(kwargs)
            body, content_type = _encode_multipart(filename, content, options)
            return self._http_multipart("/v2/parse", body, content_type)

        args: dict[str, Any] = {
            "filename": filename,
            "contentBase64": base64.b64encode(content).decode(),
        }
        if formats:
            args["formats"] = list(formats)
        if json_schema is not None:
            args["jsonSchema"] = json_schema
        if parsers:
            args["parsers"] = parsers
        args.update(kwargs)
        return self._tool_call("crw_parse_file", args)

    def extract(
        self,
        urls: list[str],
        prompt: str | None = None,
        schema: dict | None = None,
        llm_api_key: str | None = None,
        llm_provider: str | None = None,
        llm_model: str | None = None,
        poll_interval: float = 2.0,
        timeout: float = 120.0,
    ) -> list[dict]:
        """Run native multi-URL structured extraction (HTTP mode only).

        Starts an async ``/v1/extract`` job, polls until completion, and returns
        the per-URL ``results`` array (``[{url, status, data, error, llmUsage}]``)
        in request order. Requires an LLM configured on the engine (or a BYOK key).

        Args:
            urls: URLs to extract from (at least one required).
            prompt: Free-text extraction instruction (required unless ``schema``).
            schema: JSON Schema describing the desired output shape.
            llm_api_key: BYOK LLM key (self-host/local engine only).
            llm_provider: BYOK provider.
            llm_model: BYOK model.
        """
        if not self._api_url:
            raise CrwError(_HTTP_ONLY_HINT.format(name="extract", reason="LLM extract job endpoint"))

        body: dict[str, Any] = {"urls": list(urls)}
        if prompt is not None:
            body["prompt"] = prompt
        if schema is not None:
            body["schema"] = schema
        if llm_api_key is not None:
            body["llmApiKey"] = llm_api_key
        if llm_provider is not None:
            body["llmProvider"] = llm_provider
        if llm_model is not None:
            body["llmModel"] = llm_model

        start = self._http_request("POST", "/v1/extract", body, raw=True)
        job_id = start.get("id")
        if not job_id:
            # The managed API answers synchronously: the extraction is already
            # finished and the payload is in this first response, with no job to
            # poll. Only the async path hands back an `id`. Demanding one made
            # every managed extract() raise.
            if isinstance(start.get("results"), list):
                return start["results"]
            raise CrwError(f"extract returned neither a job id nor results: {start}")

        deadline = time.monotonic() + timeout
        while True:
            if time.monotonic() > deadline:
                raise CrwTimeoutError(f"Extract {job_id} timed out after {timeout}s")
            status_result = self._http_request(
                "GET", f"/v1/extract/{job_id}", raw=True, check_success=False
            )
            status = status_result.get("status")
            if status == "completed":
                return status_result.get("results", [])
            if status == "failed":
                raise CrwError(f"Extract failed: {status_result.get('error', 'unknown')}")
            time.sleep(poll_interval)

    def batch_scrape(
        self,
        urls: list[str],
        formats: list[str] | None = None,
        poll_interval: float = 2.0,
        timeout: float = 300.0,
        **kwargs: Any,
    ) -> list[dict]:
        """Scrape many URLs in one async batch job (HTTP mode only).

        Starts a ``/v2/batch/scrape`` job, polls until completion, and returns the
        list of per-URL page results.
        """
        if not self._api_url:
            raise CrwError(
                _HTTP_ONLY_HINT.format(name="batch_scrape", reason="batch job endpoint")
            )

        body: dict[str, Any] = {"urls": list(urls)}
        if formats:
            body["formats"] = list(formats)
        body.update(kwargs)

        start = self._http_request("POST", "/v2/batch/scrape", body, raw=True)
        job_id = start.get("id")
        if not job_id:
            raise CrwError(f"Batch scrape did not return job ID: {start}")

        deadline = time.monotonic() + timeout
        while True:
            if time.monotonic() > deadline:
                raise CrwTimeoutError(f"Batch scrape {job_id} timed out after {timeout}s")
            status_result = self._http_request(
                "GET", f"/v2/batch/scrape/{job_id}", raw=True, check_success=False
            )
            status = status_result.get("status")
            if status == "completed":
                return status_result.get("data", [])
            if status == "failed":
                raise CrwError(f"Batch scrape failed: {status_result.get('error', 'unknown')}")
            time.sleep(poll_interval)

    def capabilities(self) -> dict:
        """Return what this engine instance supports (HTTP mode only).

        Surfaces LLM/formats/search/document-upload capabilities from
        ``GET /v1/capabilities`` — useful for feature-detecting the server.
        """
        if not self._api_url:
            raise CrwError(
                _HTTP_ONLY_HINT.format(name="capabilities", reason="server capabilities endpoint")
            )
        return self._http_request("GET", "/v1/capabilities", check_success=False)

    def change_tracking_diff(
        self,
        current: dict,
        previous: dict | None = None,
        modes: list[str] | None = None,
        schema: dict | None = None,
        prompt: str | None = None,
        **kwargs: Any,
    ) -> dict:
        """Diff a page's current content against a prior snapshot (HTTP mode only).

        Calls the stateless ``POST /v1/change-tracking/diff`` endpoint. ``current``
        and ``previous`` are scrape-content objects, e.g. ``{"markdown": "..."}``.

        Args:
            current: Current scrape content (``{"markdown": ...}`` or ``{"json": ...}``).
            previous: Prior snapshot to diff against (caller-supplied; engine stores none).
            modes: Diff modes, defaults to ``["gitDiff"]`` (also ``"json"``).
            schema: JSON Schema for ``json`` mode structured diffs.
            prompt: Optional instruction for LLM-judged diffs.
        """
        if not self._api_url:
            raise CrwError(
                _HTTP_ONLY_HINT.format(name="change_tracking_diff", reason="diff endpoint")
            )
        body: dict[str, Any] = {"current": current, "modes": list(modes) if modes else ["gitDiff"]}
        if previous is not None:
            body["previous"] = previous
        if schema is not None:
            body["schema"] = schema
        if prompt is not None:
            body["prompt"] = prompt
        body.update(kwargs)
        return self._http_post("/v1/change-tracking/diff", body)

    def close(self) -> None:
        """Shut down the subprocess if running."""
        if self._process and self._process.poll() is None:
            if self._process.stdin:
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
        assert proc.stdin is not None and proc.stdout is not None
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

    def _http_request(
        self,
        method: str,
        path: str,
        body: dict | None = None,
        *,
        raw: bool = False,
        check_success: bool = True,
    ) -> dict:
        import urllib.request

        assert self._api_url is not None
        url = f"{self._api_url.rstrip('/')}{path}"
        headers = {"Content-Type": "application/json"}
        if self._api_key:
            headers["Authorization"] = f"Bearer {self._api_key}"

        data = json.dumps(body).encode() if body else None
        req = urllib.request.Request(url, data=data, headers=headers, method=method)

        result = _read_json_response(req)

        if check_success and not result.get("success", True):
            raise CrwApiError(result.get("error", "API error"))
        if raw:
            return result
        return result.get("data", result)

    def _http_multipart(self, path: str, body: bytes, content_type: str) -> dict:
        import urllib.request

        assert self._api_url is not None
        url = f"{self._api_url.rstrip('/')}{path}"
        headers = {"Content-Type": content_type}
        if self._api_key:
            headers["Authorization"] = f"Bearer {self._api_key}"

        req = urllib.request.Request(url, data=body, headers=headers, method="POST")
        result = _read_json_response(req)

        if not result.get("success", True):
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

            status_result = self._http_request("GET", f"/v1/crawl/{job_id}", raw=True)
            status = status_result.get("status")

            if status == "completed":
                return status_result.get("data", [])
            if status == "failed":
                raise CrwError(f"Crawl failed: {status_result.get('error', 'unknown')}")

            time.sleep(poll_interval)
