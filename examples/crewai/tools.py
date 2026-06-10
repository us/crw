"""CrewAI tools backed by CRW.

    pip install "crw[crewai]"

CRW is cloud-first: set CRW_API_KEY (sign up for 500 free credits at
https://fastcrw.com/dashboard), or set CRW_LOCAL=1 to run the engine locally.
"""

from crw.integrations.crewai import (
    CrwCrawlWebsiteTool,
    CrwMapWebsiteTool,
    CrwScrapeWebsiteTool,
    CrwSearchWebTool,
)

scrape = CrwScrapeWebsiteTool()
crawl = CrwCrawlWebsiteTool()
site_map = CrwMapWebsiteTool()
search = CrwSearchWebTool()

# Hand these to a crewai Agent(tools=[scrape, crawl, site_map, search]).
