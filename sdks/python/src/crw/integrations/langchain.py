"""CRW document loader for LangChain.

Security note: Do not pass untrusted user input to ``url``, ``api_url``,
``headers``, or ``proxy`` parameters. These are forwarded as HTTP requests
and could be used for SSRF if exposed to untrusted input.
"""

from __future__ import annotations

import os
from typing import Any, Iterator, Literal, Optional, Union

from crw.client import CrwClient

try:
    from langchain_core.document_loaders import BaseLoader
    from langchain_core.documents import Document
except ImportError as e:  # pragma: no cover
    raise ImportError(
        "crw.integrations.langchain requires the 'langchain' extra: "
        "pip install 'crw[langchain]'"
    ) from e


class CrwLoader(BaseLoader):
    """Load documents using CRW web scraper.

    CRW is a high-performance, Firecrawl-compatible web scraper written in Rust.
    Self-hosted (crw-server) or cloud (fastcrw.com).

    Setup:
        Install the package:

        .. code-block:: bash

            pip install langchain-crw

        CRW is cloud-first: by default it uses the managed cloud
        (``api.fastcrw.com``). Sign up for 500 free credits at
        https://fastcrw.com/dashboard and set ``CRW_API_KEY``. To self-host the
        engine locally instead, set ``CRW_LOCAL=1`` (zero-config, no key).

    Instantiate:
        .. code-block:: python

            from langchain_crw import CrwLoader

            # Cloud (default) — reads CRW_API_KEY from the environment
            loader = CrwLoader(url="https://example.com", mode="scrape")

            # ...or pass the key explicitly
            loader = CrwLoader(
                url="https://example.com",
                api_key="your-key",
                mode="crawl",
            )

            # Self-hosted server
            loader = CrwLoader(url="https://example.com", api_url="http://localhost:3000")

            # Local zero-config engine: set CRW_LOCAL=1 in the environment.

    Lazy load:
        .. code-block:: python

            for doc in loader.lazy_load():
                print(doc.page_content[:100])
                print(doc.metadata)
    """

    def __init__(
        self,
        url: Union[str, list[str]] = "",
        *,
        api_key: Optional[str] = None,
        api_url: Optional[str] = None,
        mode: Literal["scrape", "crawl", "map", "search", "parse", "extract"] = "scrape",
        query: Optional[str] = None,
        params: Optional[dict[str, Any]] = None,
    ) -> None:
        """Initialize CrwLoader.

        Args:
            url: The URL (or list of URLs) to scrape, crawl, map, or extract from.
                For ``parse`` mode this is the local file path to a PDF.
                Not required for search mode.
            api_key: Bearer token for authentication.
                Read from CRW_API_KEY env var if not provided.
                Not required for subprocess mode or self-hosted without auth.
            api_url: Base URL of CRW server for HTTP mode.
                Read from CRW_API_URL env var if not provided.
                Defaults to None (subprocess mode — spawns crw-mcp binary).
            mode: Operation mode - "scrape", "crawl", "map", "search", "parse"
                (local PDF → markdown/JSON), or "extract" (structured LLM
                extraction across URLs; HTTP/cloud mode only).
            query: Search query (search mode) or extraction prompt (extract mode).
            params: Additional parameters passed to the CRW API. For ``extract``,
                ``params["schema"]`` is the JSON Schema; for ``parse``,
                ``params`` may carry ``formats``/``json_schema``/``parsers``.
        """
        if mode == "search" and not query:
            raise ValueError("query is required for search mode")
        if mode != "search" and not url:
            raise ValueError(
                "url is required for scrape/crawl/map/extract modes "
                "(or a file path for parse mode)"
            )

        self.url = url
        self.query = query
        self.api_key = api_key or os.getenv("CRW_API_KEY")
        self.api_url = api_url or os.getenv("CRW_API_URL") or None
        self.mode = mode
        self.params = params or {}
        self._client: Optional[CrwClient] = None

    def _get_client(self) -> CrwClient:
        """Get or create the CrwClient instance."""
        if self._client is None:
            self._client = CrwClient(
                api_url=self.api_url,
                api_key=self.api_key,
            )
        return self._client

    def lazy_load(self) -> Iterator[Document]:
        """Lazy load documents from CRW."""
        if self.mode == "search":
            yield from self._search()
        elif self.mode == "scrape":
            yield from self._scrape()
        elif self.mode == "crawl":
            yield from self._crawl()
        elif self.mode == "map":
            yield from self._map()
        elif self.mode == "parse":
            yield from self._parse_file()
        elif self.mode == "extract":
            yield from self._extract()
        else:
            raise ValueError(
                f"Invalid mode '{self.mode}'. Must be 'scrape', 'crawl', 'map', "
                "'search', 'parse', or 'extract'."
            )

    def _scrape(self) -> Iterator[Document]:
        """Scrape one or more URLs."""
        client = self._get_client()
        kwargs = self._build_sdk_params()

        urls = [self.url] if isinstance(self.url, str) else self.url
        for u in urls:
            result = client.scrape(u, **kwargs)
            if not result:
                continue
            doc = self._parse_document(result)
            if doc.page_content:
                yield doc

    def _crawl(self) -> Iterator[Document]:
        """Crawl one or more sites."""
        client = self._get_client()
        kwargs = self._build_sdk_params()

        poll_interval = max(self.params.get("poll_interval", 2.0), 0.1)
        timeout = max(self.params.get("timeout", 300.0), 0)

        urls = [self.url] if isinstance(self.url, str) else self.url
        for u in urls:
            pages = client.crawl(
                u,
                poll_interval=poll_interval,
                timeout=timeout,
                **kwargs,
            )
            for page in pages:
                doc = self._parse_document(page)
                if doc.page_content:
                    yield doc

    def _search(self) -> Iterator[Document]:
        """Search the web. Cloud-only feature."""
        client = self._get_client()
        kwargs = self._build_sdk_params()

        results = client.search(self.query, **kwargs)

        # Flat results: list of dicts
        if isinstance(results, list):
            for result in results:
                yield Document(
                    page_content=result.get("markdown")
                    or result.get("description", ""),
                    metadata={
                        "url": result.get("url", ""),
                        "title": result.get("title", ""),
                        "score": result.get("score"),
                        "source": "search",
                    },
                )
        # Grouped results: dict with web/news/images keys
        elif isinstance(results, dict):
            for source_type, items in results.items():
                if isinstance(items, list):
                    for item in items:
                        yield Document(
                            page_content=item.get("markdown")
                            or item.get("description", ""),
                            metadata={
                                "url": item.get("url", ""),
                                "title": item.get("title", ""),
                                "source_type": source_type,
                                "source": "search",
                            },
                        )

    def _map(self) -> Iterator[Document]:
        """Discover URLs on one or more sites."""
        client = self._get_client()
        kwargs = self._build_sdk_params()

        urls = [self.url] if isinstance(self.url, str) else self.url
        for u in urls:
            links = client.map(u, **kwargs)
            for link in links:
                if isinstance(link, str) and link:
                    yield Document(page_content=link, metadata={})

    def _parse_file(self) -> Iterator[Document]:
        """Parse a local PDF file into a Document (markdown + metadata)."""
        client = self._get_client()
        paths = [self.url] if isinstance(self.url, str) else self.url
        parse_keys = {"formats", "json_schema", "parsers"}
        kwargs = {k: v for k, v in self.params.items() if k in parse_keys}
        for path in paths:
            result = client.parse_file(path, **kwargs)
            if result:
                doc = self._parse_document(result)
                if doc.page_content:
                    yield doc

    def _extract(self) -> Iterator[Document]:
        """Structured LLM extraction across URLs (HTTP/cloud mode only)."""
        import json

        client = self._get_client()
        urls = [self.url] if isinstance(self.url, str) else self.url
        data = client.extract(
            urls,
            prompt=self.query,
            schema=self.params.get("schema"),
            system_prompt=self.params.get("system_prompt"),
        )
        yield Document(
            page_content=json.dumps(data, ensure_ascii=False),
            metadata={"source": "extract", "urls": list(urls)},
        )

    def _build_sdk_params(self) -> dict[str, Any]:
        """Build keyword arguments for CrwClient methods.

        Passes params through as snake_case kwargs. The CrwClient SDK
        handles camelCase conversion internally.
        """
        # Keys handled separately (not forwarded to SDK methods)
        skip_keys = {"poll_interval", "timeout"}

        result: dict[str, Any] = {}
        for key, value in self.params.items():
            if key not in skip_keys:
                result[key] = value
        return result

    def close(self) -> None:
        """Clean up the underlying CrwClient."""
        if getattr(self, "_client", None) is not None:
            self._client.close()
            self._client = None

    def __del__(self) -> None:
        self.close()

    @staticmethod
    def _parse_document(page: dict[str, Any]) -> Document:
        """Convert a CRW page response to a LangChain Document."""
        content = (
            page.get("markdown")
            or page.get("html")
            or page.get("rawHtml")
            or page.get("plainText")
            or ""
        )
        # Ensure content is a string
        if not isinstance(content, str):
            content = str(content)
        metadata = page.get("metadata") or {}
        if not isinstance(metadata, dict):
            metadata = {}
        return Document(page_content=content, metadata=metadata)
