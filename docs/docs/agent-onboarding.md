# Agent Onboarding Guide

## Overview

This guide is written for AI agents and the systems that orchestrate them. It covers the minimum you need to start making API calls, the most common patterns, and how to handle errors gracefully.

## Authentication

Every request requires an API key in the `Authorization` header:

```
Authorization: Bearer YOUR_API_KEY
```

:::note
On self-hosted instances, authentication is configured via your server settings. Cloud only (fastcrw.com) provides a dashboard for key management.
:::

## Available Endpoints

| Endpoint | Method | Purpose |
| --- | --- | --- |
| `/v1/scrape` | POST | Extract content from a single URL |
| `/v1/crawl` | POST | Recursively collect pages from a domain |
| `/v1/map` | POST | Discover all reachable URLs on a domain |
| `/v1/search` | POST | Search the web and return results with content |

## Quick Start Pattern

For most agent workflows, start with this sequence:

1. **Discover** what pages exist on a target domain:

```bash
curl -X POST https://fastcrw.com/api/v1/map \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'
```

2. **Extract** content from specific pages:

```bash
curl -X POST https://fastcrw.com/api/v1/scrape \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com/page", "formats": ["markdown"]}'
```

3. **Search** when you need to find relevant pages across the web:

```bash
curl -X POST https://fastcrw.com/api/v1/search \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"query": "your search query", "limit": 5}'
```

## Response Format

All endpoints return JSON with a consistent structure:

```json
{
  "success": true,
  "data": { ... }
}
```

On failure:

```json
{
  "success": false,
  "error": "description of what went wrong"
}
```

## Error Handling for Agents

Agents should handle these common scenarios:

| Status | Meaning | Agent action |
| --- | --- | --- |
| 200 | Success | Process the response |
| 400 | Bad request | Fix the request parameters |
| 401 | Invalid API key | Check authentication |
| 429 | Rate limited or out of credits | Back off and retry, or alert the user |
| 500 | Server error | Retry with exponential backoff |

## Common Agent Patterns

### Research Loop

```
search(topic) -> scrape(top results) -> analyze -> search(refined query) -> repeat
```

### Site Exploration

```
map(domain) -> filter URLs -> scrape(relevant pages) -> synthesize
```

### Monitoring

```
scrape(url) -> store result -> wait -> scrape(url) -> compare with previous
```

## Rate Limits

Check the [rate limits documentation](/docs/rate-limits) for current limits. Agents should respect `Retry-After` headers and implement exponential backoff.

## MCP Integration

If your agent runtime supports MCP, see the [MCP guide](/docs/mcp) for a simpler integration path that avoids direct HTTP calls.
