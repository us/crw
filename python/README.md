# crw

Python SDK for [CRW](https://github.com/us/crw) — the open-source web scraper built for AI agents.

## Install

```bash
# npm (zero install):
npx crw-mcp

# Python:
pip install crw

# Direct binary (no package manager):
curl -fsSL https://github.com/us/crw/releases/latest/download/crw-mcp-darwin-arm64.tar.gz | tar xz
# Replace darwin-arm64 with your platform: darwin-x64, linux-x64, linux-arm64, win32-x64, win32-arm64

# Cargo:
cargo install crw-mcp

# Docker:
docker run -i ghcr.io/us/crw crw-mcp
```

## CLI Usage

After installing, you can use `crw-mcp` as an MCP server for any AI coding agent:

```bash
# Start the MCP stdio server
crw-mcp

# Add to Claude Code
claude mcp add crw -- npx crw-mcp
```

MCP client config (works with Cursor, Windsurf, Cline, Claude Desktop, etc.):

```json
{
  "mcpServers": {
    "crw": {
      "command": "npx",
      "args": ["crw-mcp"]
    }
  }
}
```

## SDK Usage

```python
from crw import CrwClient

# Zero-config (downloads crw-mcp binary automatically):
client = CrwClient()
result = client.scrape("https://example.com")
print(result["markdown"])

# Or connect to a remote server:
client = CrwClient(api_url="https://fastcrw.com/api", api_key="fc-...")

# Scrape with options:
result = client.scrape("https://example.com", formats=["markdown", "links"])
print(result["markdown"])
print(result["links"])

# Crawl a site:
job = client.crawl("https://example.com", max_depth=2, max_pages=10)
print(job["id"])

# Map all URLs on a site:
urls = client.map("https://example.com")
print(urls)
```
