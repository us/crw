# Framework Integrations

CRW integrates with popular AI agent frameworks and workflow tools. Since CRW exposes a Firecrawl-compatible REST API, it also works as a drop-in replacement anywhere Firecrawl is used.

## CrewAI

Published PyPI package: [`crewai-crw`](https://pypi.org/project/crewai-crw/)

```bash
pip install crewai crewai-crw
```

Three ready-to-use tools — no custom `BaseTool` subclasses needed:

```python
from crewai import Agent, Task, Crew
from crewai_crw import CrwScrapeWebsiteTool, CrwCrawlWebsiteTool, CrwMapWebsiteTool

# Self-hosted (default: localhost:3000)
scrape_tool = CrwScrapeWebsiteTool()

# Or use fastCRW cloud
scrape_tool = CrwScrapeWebsiteTool(
    api_url="https://fastcrw.com/api",
    api_key="your-api-key",
)

# Or set env vars
# export CRW_API_URL=https://fastcrw.com/api
# export CRW_API_KEY=your-api-key

researcher = Agent(
    role="Web Researcher",
    goal="Research and summarize information from websites",
    tools=[scrape_tool],
)

task = Task(
    description="Scrape https://example.com and summarize the content",
    expected_output="A summary of the page content",
    agent=researcher,
)

crew = Crew(agents=[researcher], tasks=[task])
result = crew.kickoff()
```

Source: [github.com/us/crewai-crw](https://github.com/us/crewai-crw)

## LangChain (PR pending)

Community document loader submitted: [PR #606](https://github.com/langchain-ai/langchain-community/pull/606)

In the meantime, use the HTTP API directly:

```python
import requests

def load_documents(urls):
    documents = []
    for url in urls:
        resp = requests.post("http://localhost:3000/v1/scrape", json={
            "url": url,
            "formats": ["markdown"]
        })
        data = resp.json()["data"]
        documents.append({
            "page_content": data["markdown"],
            "metadata": data["metadata"]
        })
    return documents
```

## Flowise (PR pending)

CRW node submitted: [PR #6066](https://github.com/FlowiseAI/Flowise/pull/6066)

Provides Scrape, Crawl, and Map nodes that connect to your CRW instance. Configure the CRW API URL in the node settings.

## Agno (PR pending)

CRW toolkit submitted: [PR #7183](https://github.com/agno-agi/agno/pull/7183)

Once merged, usage will be:

```python
from agno.agent import Agent
from agno.tools.crw import CrwTools

agent = Agent(tools=[CrwTools()])
agent.print_response("Scrape https://example.com and summarize it")
```

## n8n

Use n8n's HTTP Request nodes to connect to CRW's REST API. No custom node package required — CRW's endpoints work directly with n8n's built-in HTTP nodes.

See the [n8n tutorial](https://fastcrw.com/blog/n8n-web-scraping-crw) for step-by-step setup.

## MCP (10+ Platforms)

CRW includes a built-in MCP server that works with any MCP-compatible platform. See the [MCP documentation](#mcp) for setup instructions for:

- Claude Code, Claude Desktop
- Cursor, Windsurf
- Cline, Continue.dev
- OpenAI Codex CLI
- Gemini CLI
- VS Code GitHub Copilot Agent
- Roo Code

## Firecrawl Drop-in Replacement

CRW is API-compatible with Firecrawl. Any project that uses `FIRECRAWL_BASE_URL` or similar config can switch to CRW with zero code changes:

```bash
# Point Firecrawl SDK at your CRW instance
export FIRECRAWL_API_URL=http://localhost:3000
```

```python
from firecrawl import FirecrawlApp

# This talks to CRW, not Firecrawl
app = FirecrawlApp(api_url="http://localhost:3000")
result = app.scrape_url("https://example.com")
```

> **Note:** The Firecrawl Python SDK has changed its API across versions. The above works with `firecrawl-py` v1.x. Check the [Firecrawl docs](https://docs.firecrawl.dev) for the latest SDK usage.

## Python (Direct HTTP)

```python
import requests

response = requests.post("http://localhost:3000/v1/scrape", json={
    "url": "https://example.com",
    "formats": ["markdown", "links"]
})
data = response.json()["data"]
print(data["markdown"])
```

## Node.js (Direct HTTP)

```javascript
const response = await fetch("http://localhost:3000/v1/scrape", {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({
    url: "https://example.com",
    formats: ["markdown", "links"]
  })
});
const { data } = await response.json();
console.log(data.markdown);
```

## All Integrations

| Framework | Type | Status | Package / PR |
|-----------|------|--------|-------------|
| [CrewAI](https://github.com/crewAIInc/crewAI) | PyPI package | **Published** | [`crewai-crw`](https://pypi.org/project/crewai-crw/) |
| [LangChain](https://github.com/langchain-ai/langchain) | Community loader | PR pending | [#606](https://github.com/langchain-ai/langchain-community/pull/606) |
| [Flowise](https://github.com/FlowiseAI/Flowise) | Node | PR pending | [#6066](https://github.com/FlowiseAI/Flowise/pull/6066) |
| [Agno](https://github.com/agno-agi/agno) | Toolkit | PR pending | [#7183](https://github.com/agno-agi/agno/pull/7183) |
| [n8n](https://github.com/n8n-io/n8n) | HTTP nodes | Works now | [Tutorial](https://fastcrw.com/blog/n8n-web-scraping-crw) |
| MCP (10+ platforms) | Built-in | **Shipped** | [MCP docs](#mcp) |
| Firecrawl SDK | Drop-in | **Works now** | API compatible |
