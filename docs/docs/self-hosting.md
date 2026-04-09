# Self-Hosting

Use self-hosting when you want CRW running inside your own infrastructure. The hosted API is still the fastest first-run path; self-hosting is the better fit when network control, auth, and deployment ownership matter more than zero-setup speed.

## Quick Start

The shortest self-hosted path is Docker:

```bash
docker run -p 3000:3000 ghcr.io/us/crw
```

Then make a local request:

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{"url":"https://example.com","formats":["markdown"]}'
```

## What You Get

- the same core self-hosted routes: `scrape`, `crawl`, `map`, `mcp`, `health`
- optional auth with Bearer tokens
- optional browser-backed rendering
- your own reverse proxy, logging, rate limits, and deployment choices

## Recommended Rollout

1. start with `scrape` on a real target
2. validate one `map` request
3. run a bounded `crawl` with a low page cap
4. add auth, TLS, and edge controls before broader access
5. add JS rendering only when targets actually need it

## When Self-Hosting Is The Right Choice

Choose self-hosting when you want:

- private network placement,
- direct control over operational cost,
- custom auth and routing,
- or a local MCP/server flow without depending on the hosted product.

## When The Hosted API Is Better

Choose the hosted path when:

- you want your first successful request immediately,
- you do not want to manage renderer dependencies,
- or your priority is product velocity rather than infrastructure ownership.

## Common Mistakes

- Starting with browser rendering before validating plain HTTP scraping
- Exposing the service publicly before adding auth, TLS, and rate limits
- Treating one happy-path scrape as enough deployment validation

## What To Read Next

- [Installation](#installation)
- [Configuration](#configuration)
- [Docker](#docker)
- [Self-Hosting Hardening](#self-hosting-hardening)
