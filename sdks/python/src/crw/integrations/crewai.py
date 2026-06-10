"""CRW web scraping tools for CrewAI.

CRW is an open-source web scraper built for AI agents.
Self-hosted (free) or managed cloud at https://fastcrw.com.
"""

from __future__ import annotations

from typing import Any

from crw.client import CrwClient

try:
    from crewai.tools import BaseTool
    from pydantic import BaseModel, ConfigDict, Field
except ImportError as e:  # pragma: no cover
    raise ImportError(
        "crw.integrations.crewai requires the 'crewai' extra: "
        "pip install 'crw[crewai]'"
    ) from e


def _make_client(api_url: str | None, api_key: str | None) -> CrwClient:
    """Create a CrwClient instance.

    CRW is cloud-first: with no api_url and no key the SDK targets the managed
    cloud (sign up for 500 free credits, set CRW_API_KEY). Pass an api_url for a
    self-hosted server, or set CRW_LOCAL=1 to run the local engine.
    """
    return CrwClient(api_url=api_url, api_key=api_key)


# --- Scrape Tool ---


class CrwScrapeWebsiteToolSchema(BaseModel):
    url: str = Field(description="Website URL to scrape")


class CrwScrapeWebsiteTool(BaseTool):
    """Scrape webpages using CRW and return clean markdown content.

    CRW is an open-source web scraper built for AI agents. It can run
    self-hosted (free) or via the managed cloud at fastcrw.com.

    Args:
        api_url: CRW server URL for a self-hosted server. If None, uses the
            managed cloud by default (set CRW_LOCAL=1 to run locally instead).
        api_key: Cloud API key (or CRW_API_KEY env). Sign up for 500 free credits
            at fastcrw.com/dashboard.
        config: Scrape configuration options.

    Configuration options:
        formats (list[str]): Output formats. Default: ["markdown"]
            Options: "markdown", "html", "rawHtml", "plainText", "links", "json"
        only_main_content (bool): Strip nav/footer/sidebar. Default: True
        render_js (bool|None): None=auto, True=force JS, False=HTTP only.
        wait_for (int): ms to wait after JS rendering.
        include_tags (list[str]): CSS selectors to include.
        exclude_tags (list[str]): CSS selectors to exclude.
        css_selector (str): Extract specific CSS selector.
        xpath (str): Extract specific XPath.
        headers (dict): Custom HTTP headers.
        json_schema (dict): JSON Schema for LLM extraction.
        proxy (str): Per-request proxy URL.
        stealth (bool): Enable stealth mode.
    """

    model_config = ConfigDict(
        arbitrary_types_allowed=True, validate_assignment=True, frozen=False
    )
    name: str = "CRW web scrape tool"
    description: str = (
        "Scrape webpages using CRW and return clean markdown content. "
        "CRW is a fast, open-source web scraper with JS rendering support."
    )
    args_schema: type[BaseModel] = CrwScrapeWebsiteToolSchema
    api_url: str | None = None
    api_key: str | None = None
    config: dict[str, Any] = Field(
        default_factory=lambda: {
            "formats": ["markdown"],
            "onlyMainContent": True,
        }
    )
    _client: CrwClient | None = None

    def __init__(
        self,
        api_url: str | None = None,
        api_key: str | None = None,
        **kwargs: Any,
    ) -> None:
        super().__init__(**kwargs)
        self.api_url = api_url
        self.api_key = api_key
        self._client = _make_client(api_url, api_key)

    def _run(self, url: str) -> Any:
        formats = self.config.get("formats")
        only_main_content = self.config.get("onlyMainContent", True)

        # Pass remaining config keys as extra kwargs
        extra: dict[str, Any] = {
            k: v
            for k, v in self.config.items()
            if k not in ("formats", "onlyMainContent")
        }

        result = self._client.scrape(
            url,
            formats=formats,
            only_main_content=only_main_content,
            **extra,
        )

        if result.get("markdown"):
            return result["markdown"]
        if result.get("plainText"):
            return result["plainText"]
        if result.get("html"):
            return result["html"]
        if result.get("json"):
            return result["json"]
        return result


# --- Crawl Tool ---


class CrwCrawlWebsiteToolSchema(BaseModel):
    url: str = Field(description="Website URL to crawl")


class CrwCrawlWebsiteTool(BaseTool):
    """Crawl websites using CRW and return content from multiple pages.

    CRW performs async BFS crawling with rate limiting, robots.txt respect,
    and sitemap support. Runs self-hosted (free) or via fastcrw.com cloud.

    Args:
        api_url: CRW server URL for a self-hosted server. If None, uses the
            managed cloud by default (set CRW_LOCAL=1 to run locally instead).
        api_key: Cloud API key (or CRW_API_KEY env). Sign up for 500 free credits
            at fastcrw.com/dashboard.
        config: Crawl configuration options.
        poll_interval: Seconds between status checks. Default: 2
        max_wait: Maximum seconds to wait for crawl completion. Default: 300

    Configuration options:
        max_depth (int): Maximum link-follow depth. Default: 2
        max_pages (int): Maximum pages to scrape. Default: 10
        formats (list[str]): Output formats per page. Default: ["markdown"]
        only_main_content (bool): Strip boilerplate. Default: True
    """

    model_config = ConfigDict(
        arbitrary_types_allowed=True, validate_assignment=True, frozen=False
    )
    name: str = "CRW web crawl tool"
    description: str = (
        "Crawl websites using CRW and return content from multiple pages. "
        "Useful for gathering information across an entire site."
    )
    args_schema: type[BaseModel] = CrwCrawlWebsiteToolSchema
    api_url: str | None = None
    api_key: str | None = None
    poll_interval: int = 2
    max_wait: int = 300
    config: dict[str, Any] = Field(
        default_factory=lambda: {
            "maxDepth": 2,
            "maxPages": 10,
            "formats": ["markdown"],
            "onlyMainContent": True,
        }
    )
    _client: CrwClient | None = None

    def __init__(
        self,
        api_url: str | None = None,
        api_key: str | None = None,
        **kwargs: Any,
    ) -> None:
        super().__init__(**kwargs)
        self.api_url = api_url
        self.api_key = api_key
        self._client = _make_client(api_url, api_key)

    def _run(self, url: str) -> Any:
        max_depth = self.config.get("maxDepth", 2)
        max_pages = self.config.get("maxPages", 10)

        # Pass remaining config keys as extra kwargs
        extra: dict[str, Any] = {
            k: v
            for k, v in self.config.items()
            if k not in ("maxDepth", "maxPages")
        }

        pages = self._client.crawl(
            url,
            max_depth=max_depth,
            max_pages=max_pages,
            poll_interval=float(self.poll_interval),
            timeout=float(self.max_wait),
            **extra,
        )

        combined = []
        for page in pages:
            source = page.get("metadata", {}).get("sourceURL", "unknown")
            content = page.get("markdown", "")
            if content:
                combined.append(f"## Source: {source}\n\n{content}")
        return "\n\n---\n\n".join(combined) if combined else "No content found."


# --- Map Tool ---


class CrwMapWebsiteToolSchema(BaseModel):
    url: str = Field(description="Website URL to discover links from")


class CrwMapWebsiteTool(BaseTool):
    """Discover all URLs on a website using CRW's map endpoint.

    Useful for understanding site structure before targeted scraping.
    Uses sitemap.xml and link discovery to find all pages.

    Args:
        api_url: CRW server URL for a self-hosted server. If None, uses the
            managed cloud by default (set CRW_LOCAL=1 to run locally instead).
        api_key: Cloud API key (or CRW_API_KEY env). Sign up for 500 free credits
            at fastcrw.com/dashboard.
        config: Map configuration options.

    Configuration options:
        max_depth (int): Maximum discovery depth. Default: 2
        use_sitemap (bool): Also read sitemap.xml. Default: True
    """

    model_config = ConfigDict(
        arbitrary_types_allowed=True, validate_assignment=True, frozen=False
    )
    name: str = "CRW website map tool"
    description: str = (
        "Discover all URLs on a website. Useful for understanding site structure "
        "before scraping specific pages."
    )
    args_schema: type[BaseModel] = CrwMapWebsiteToolSchema
    api_url: str | None = None
    api_key: str | None = None
    config: dict[str, Any] = Field(
        default_factory=lambda: {
            "maxDepth": 2,
            "useSitemap": True,
        }
    )
    _client: CrwClient | None = None

    def __init__(
        self,
        api_url: str | None = None,
        api_key: str | None = None,
        **kwargs: Any,
    ) -> None:
        super().__init__(**kwargs)
        self.api_url = api_url
        self.api_key = api_key
        self._client = _make_client(api_url, api_key)

    def _run(self, url: str) -> Any:
        max_depth = self.config.get("maxDepth", 2)
        use_sitemap = self.config.get("useSitemap", True)

        # Pass remaining config keys as extra kwargs
        extra: dict[str, Any] = {
            k: v
            for k, v in self.config.items()
            if k not in ("maxDepth", "useSitemap")
        }

        links = self._client.map(
            url,
            max_depth=max_depth,
            use_sitemap=use_sitemap,
            **extra,
        )

        return "\n".join(links) if links else "No links discovered."


# --- Search Tool ---


class CrwSearchWebToolSchema(BaseModel):
    query: str = Field(description="Search query to find web content")


class CrwSearchWebTool(BaseTool):
    """Search the web using CRW. Cloud-only feature — requires api_url."""

    model_config = ConfigDict(
        arbitrary_types_allowed=True, validate_assignment=True, frozen=False
    )

    name: str = "CRW web search tool"
    description: str = (
        "Search the web using CRW and return relevant results with titles, "
        "descriptions, and optionally scraped content. Cloud-only feature — "
        "requires api_url (e.g. https://fastcrw.com/api)."
    )
    args_schema: type[BaseModel] = CrwSearchWebToolSchema
    api_url: str | None = None
    api_key: str | None = None
    config: dict[str, Any] = Field(default_factory=lambda: {"limit": 5})
    _client: CrwClient | None = None

    def __init__(
        self,
        api_url: str | None = None,
        api_key: str | None = None,
        **kwargs: Any,
    ) -> None:
        super().__init__(**kwargs)
        self.api_url = api_url
        self.api_key = api_key
        self._client = _make_client(api_url, api_key)

    def _run(self, query: str) -> Any:
        limit = self.config.get("limit", 5)
        extra: dict[str, Any] = {
            k: v for k, v in self.config.items() if k != "limit"
        }

        results = self._client.search(query, limit=limit, **extra)

        if isinstance(results, list):
            formatted = []
            for r in results:
                title = r.get("title", "")
                url = r.get("url", "")
                desc = r.get("description", "")
                content = r.get("markdown", "")
                entry = f"## {title}\nURL: {url}\n{desc}"
                if content:
                    entry += f"\n\n{content}"
                formatted.append(entry)
            return "\n\n---\n\n".join(formatted) if formatted else "No results found."

        return str(results)
