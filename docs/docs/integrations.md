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

## LangChain

Published PyPI package: [`langchain-crw`](https://pypi.org/project/langchain-crw/)

```bash
pip install langchain-crw
```

```python
from langchain_crw import CrwLoader

# Self-hosted (default: localhost:3000)
loader = CrwLoader(url="https://example.com", mode="scrape")
docs = loader.load()

# Cloud (fastcrw.com)
loader = CrwLoader(
    url="https://example.com",
    api_url="https://fastcrw.com/api",
    api_key="crw_live_...",
    mode="crawl",
    params={"max_depth": 3, "max_pages": 50},
)
docs = loader.load()
```

Source: [github.com/us/langchain-crw](https://github.com/us/langchain-crw)

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

## OpenClaw

Published npm package: [`openclaw-plugin-crw`](https://www.npmjs.com/package/openclaw-plugin-crw)

```bash
openclaw plugins install openclaw-plugin-crw
```

```json
{
  "plugins": {
    "crw": {
      "apiKey": "crw_live_..."
    }
  }
}
```

Cloud is the default. For self-hosted, add `"apiUrl": "http://localhost:3000"`.

Source: [github.com/us/openclaw-plugin-crw](https://github.com/us/openclaw-plugin-crw)

## n8n

Published npm package: [`n8n-nodes-crw`](https://www.npmjs.com/package/n8n-nodes-crw)

Install via n8n UI: **Settings > Community Nodes > Install > `n8n-nodes-crw`**

Or via Docker:
```bash
docker run -e EXTRA_COMMUNITY_PACKAGES=n8n-nodes-crw n8nio/n8n
```

Source: [github.com/us/n8n-nodes-crw](https://github.com/us/n8n-nodes-crw)

## MCP (10+ Platforms)

CRW includes a built-in MCP server that works with any MCP-compatible platform. See the [MCP documentation](/docs/mcp) for setup instructions for:

- Claude Code, Claude Desktop
- Cursor, Windsurf
- Cline, Continue.dev
- OpenAI Codex CLI
- Gemini CLI
- VS Code GitHub Copilot Agent
- Roo Code

## Automation Tools

| Tool | Integration path | Notes |
|------|-----------------|-------|
| n8n | Community node | [`n8n-nodes-crw`](https://www.npmjs.com/package/n8n-nodes-crw) |
| Make (Integromat) | HTTP module | Standard REST API integration |
| Zapier | Webhooks | Use webhook triggers with the API |
| GitHub Actions | curl in workflow | Useful for scheduled scraping jobs |

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

## Building Custom Integrations

The API is straightforward enough that most integrations are a thin wrapper:

1. Set the `Authorization` header with your API key.
2. POST JSON to the endpoint you need (`/v1/scrape`, `/v1/crawl`, `/v1/map`).
3. Parse the JSON response.

No SDK is required — the consistent API design means any HTTP client works.

## Choosing Between Cloud and Self-Hosted

| Factor | Cloud | Self-hosted |
|--------|-------|-------------|
| Setup time | Instant | 5-10 minutes |
| Maintenance | Managed | You handle updates |
| Data residency | Managed infrastructure. Available on [fastcrw.com](https://fastcrw.com) (cloud) | Your infrastructure |
| Cost model | Credit-based | Your server costs only |
| Rate limits | Per plan | Unlimited |

Both options expose the same API, so your integration code works with either.

## All Integrations

| Framework | Type | Status | Package / PR |
|-----------|------|--------|-------------|
| [CrewAI](https://github.com/crewAIInc/crewAI) | PyPI package | **Published** | [`crewai-crw`](https://pypi.org/project/crewai-crw/) |
| [LangChain](https://github.com/langchain-ai/langchain) | PyPI package | **Published** | [`langchain-crw`](https://pypi.org/project/langchain-crw/) |
| [OpenClaw](https://github.com/openclaw/openclaw) | npm plugin | **Published** | [`openclaw-plugin-crw`](https://www.npmjs.com/package/openclaw-plugin-crw) |
| [n8n](https://github.com/n8n-io/n8n) | npm node | **Published** | [`n8n-nodes-crw`](https://www.npmjs.com/package/n8n-nodes-crw) |
| [Flowise](https://github.com/FlowiseAI/Flowise) | Node | PR pending | [#6066](https://github.com/FlowiseAI/Flowise/pull/6066) |
| [Agno](https://github.com/agno-agi/agno) | Toolkit | PR pending | [#7183](https://github.com/agno-agi/agno/pull/7183) |
| [Dify](https://github.com/langgenius/dify) | Plugin | Ready | [GitHub](https://github.com/us/dify-plugin-crw) |
| MCP (10+ platforms) | Built-in | **Shipped** | [MCP docs](/docs/mcp) |
| Firecrawl SDK | Drop-in | **Works now** | API compatible |
