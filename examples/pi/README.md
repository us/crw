# CRW + Pi (via MCP)

Pi speaks MCP, so point it at the `crw-mcp` server — no bespoke package needed.

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

Sign up for 500 free credits at https://fastcrw.com/dashboard. Tools:
`crw_scrape`, `crw_crawl`, `crw_check_crawl_status`, `crw_map`, `crw_search`,
`crw_parse_file`.
