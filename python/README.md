# crw

Python SDK for [CRW](https://github.com/us/crw) — the open-source web scraper built for AI agents.

## Install

```bash
pip install crw
```

## Usage

```python
from crw import CrwClient

# Zero-config (downloads crw-mcp binary automatically):
client = CrwClient()
result = client.scrape("https://example.com")
print(result["markdown"])

# Or connect to a remote server:
client = CrwClient(api_url="https://fastcrw.com/api", api_key="fc-...")
```

## MCP Server

After installing, you can also use `crw-mcp` as an MCP server:

```bash
crw-mcp  # starts stdio MCP server
```
