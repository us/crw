"""LangChain document loading with CRW.

    pip install "crw[langchain]"

CRW is cloud-first: set CRW_API_KEY (sign up for 500 free credits at
https://fastcrw.com/dashboard), or set CRW_LOCAL=1 to run the engine locally.
"""

from crw.integrations.langchain import CrwLoader

# Scrape a single page into a LangChain Document
docs = CrwLoader(url="https://example.com", mode="scrape").load()
print(docs[0].page_content[:200])

# Crawl a site
docs = CrwLoader(url="https://example.com", mode="crawl", params={"maxPages": 10}).load()

# Parse a local PDF (mode="parse"), or structured extraction (mode="extract")
docs = CrwLoader(url="report.pdf", mode="parse").load()
