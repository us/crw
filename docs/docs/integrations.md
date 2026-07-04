# Framework Integrations

CRW integrates with popular AI agent frameworks and workflow tools. New integrations should use CRW's native `/v1` API. Existing Firecrawl v2 SDK integrations can target the `/firecrawl/v2` compatibility layer after validating the documented differences.

## CrewAI

CrewAI support is bundled as an **extra** in the `crw` Python package — no separate package needed.

```bash
pip install "crw[crewai]"
```

Four ready-to-use tools — no custom `BaseTool` subclasses needed:

```python
from crewai import Agent, Task, Crew
from crw.integrations.crewai import (
    CrwScrapeWebsiteTool,
    CrwCrawlWebsiteTool,
    CrwMapWebsiteTool,
    CrwSearchWebTool,
)

# Self-hosted (default: localhost:3000)
scrape_tool = CrwScrapeWebsiteTool()

# Or use fastCRW cloud
scrape_tool = CrwScrapeWebsiteTool(
    api_url="https://api.fastcrw.com",
    api_key="crw_live_...",
)

# Or set env vars
# export CRW_API_URL=https://api.fastcrw.com
# export CRW_API_KEY=crw_live_...

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

## LangChain

LangChain support is bundled as an **extra** in the `crw` Python package — no separate package needed.

```bash
pip install "crw[langchain]"
```

```python
from crw.integrations.langchain import CrwLoader

# Self-hosted (default: localhost:3000)
loader = CrwLoader(url="https://example.com", mode="scrape")
docs = loader.load()

# Cloud (fastcrw.com)
loader = CrwLoader(
    url="https://example.com",
    api_url="https://api.fastcrw.com",
    api_key="crw_live_...",
    mode="crawl",
    params={"max_depth": 3, "max_pages": 50},
)
docs = loader.load()

# Search the web (cloud only)
loader = CrwLoader(
    mode="search",
    query="web scraping tools",
    api_url="https://api.fastcrw.com",
    api_key="crw_live_...",
)
docs = loader.load()
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

## OpenClaw

OpenClaw integrates with CRW via the **MCP server** — no separate plugin package is needed. Point OpenClaw at the `crw-mcp` binary and it gains Scrape, Crawl, Map, and Search tools automatically.

See the [MCP documentation](/docs/mcp) for configuration details.

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
- OpenClaw
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

## Firecrawl v2 SDK Migration

For existing Firecrawl v2 SDK projects, point the SDK at a CRW engine and validate the compatibility matrix before switching production traffic:

```bash
# Point the Firecrawl SDK at your CRW instance
export FIRECRAWL_API_URL=http://localhost:3000
```

```python
from firecrawl import FirecrawlApp

# This talks to CRW, not Firecrawl
app = FirecrawlApp(api_url="http://localhost:3000")
result = app.scrape_url("https://example.com")
```

> **Note:** This is a migration path, not the default recommendation for new CRW code. New integrations should use the CRW SDKs or direct `/v1` HTTP routes.

## Python (Direct HTTP)

```python
import requests

response = requests.post("https://api.fastcrw.com/v1/scrape", json={
    "url": "https://example.com",
    "formats": ["markdown", "links"]
})
data = response.json()["data"]
print(data["markdown"])
```

## Node.js (Direct HTTP)

```javascript
const response = await fetch("https://api.fastcrw.com/v1/scrape", {
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
2. POST JSON to the endpoint you need (`/v1/scrape`, `/v1/crawl`, `/v1/map`, `/v1/search`).
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

## Endpoint Support Matrix

Not every integration supports every endpoint. Search requires a cloud API backend or a SearXNG sidecar configured for the embedded MCP server.

| Integration | Scrape | Crawl | Map | Search | Extract |
|-------------|--------|-------|-----|--------|---------|
| [CrewAI](https://pypi.org/project/crw/) (`crw[crewai]`) | Yes | Yes | Yes | Yes (cloud) | -- |
| [LangChain](https://pypi.org/project/crw/) (`crw[langchain]`) | Yes | Yes | Yes | Yes (cloud) | -- |
| [n8n](https://www.npmjs.com/package/n8n-nodes-crw) | Yes | Yes | Yes | Yes (cloud) | -- |
| [Dify](https://github.com/us/dify-plugin-crw) | Yes | Yes | Yes | Yes (cloud) | -- |
| MCP Server (proxy mode) | Yes | Yes | Yes | Yes | -- |
| MCP Server (embedded, no SearXNG) | Yes | Yes | Yes | -- | -- |
| Firecrawl v2 SDK migration | Yes | Yes | Yes | -- | Validate compatibility first |
| Direct HTTP | Yes | Yes | Yes | Yes (cloud) | Yes |

:::note
In MCP **proxy mode** (`--api-url` / `CRW_API_URL` set), `crw_search` is always advertised and the remote server handles it. In **embedded mode**, `crw_search` is only available when a SearXNG sidecar is configured.
:::

## All Integrations

| Framework | Type | Status | Package / PR |
|-----------|------|--------|-------------|
| [CrewAI](https://github.com/crewAIInc/crewAI) | Python extra | **Published** | [`crw[crewai]`](https://pypi.org/project/crw/) |
| [LangChain](https://github.com/langchain-ai/langchain) | Python extra | **Published** | [`crw[langchain]`](https://pypi.org/project/crw/) |
| [OpenClaw](https://github.com/openclaw/openclaw) | via MCP | **Works now** | [MCP docs](/docs/mcp) |
| [n8n](https://github.com/n8n-io/n8n) | npm node | **Published** | [`n8n-nodes-crw`](https://www.npmjs.com/package/n8n-nodes-crw) |
| [Flowise](https://github.com/FlowiseAI/Flowise) | Node | PR pending | [#6066](https://github.com/FlowiseAI/Flowise/pull/6066) |
| [Agno](https://github.com/agno-agi/agno) | Toolkit | PR pending | [#7183](https://github.com/agno-agi/agno/pull/7183) |
| [Dify](https://github.com/langgenius/dify) | Plugin | Ready | [`dify-plugin-crw`](https://github.com/us/dify-plugin-crw) |
| MCP (10+ platforms) | Built-in | **Shipped** | [MCP docs](/docs/mcp) |
| Firecrawl SDK | Migration via `/firecrawl/v2` | **Works now** | Compatibility layer |
