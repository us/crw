# CRW + OpenClaw (via MCP)

OpenClaw speaks MCP, so there's no bespoke package — point it at the `crw-mcp`
server. Install it with `npm i -g crw-mcp` (or `npx crw-mcp`).

Add to your OpenClaw MCP config:

```json
{
  "mcpServers": {
    "crw": {
      "command": "npx",
      "args": ["-y", "crw-mcp"],
      "env": { "CRW_API_KEY": "crw_live_..." }
    }
  }
}
```

Sign up for 500 free credits at https://fastcrw.com/dashboard, or omit
`CRW_API_KEY` and set `CRW_LOCAL=1`-style local config per the crw-mcp docs.

Tools exposed: `crw_scrape`, `crw_crawl`, `crw_check_crawl_status`, `crw_map`,
`crw_search`, `crw_parse_file`.
