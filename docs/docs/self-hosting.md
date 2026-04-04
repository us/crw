# Self-Hosting Guide

## Quick Start

The fastest way to get CRW running locally -- the install script auto-detects your OS and architecture:

```bash
curl -fsSL https://raw.githubusercontent.com/us/crw/main/install.sh | sh
```

This downloads the latest `crw-mcp` binary for your platform (macOS Intel/Apple Silicon, Linux x64/ARM64) and installs it to `/usr/local/bin`. You can customize the install directory:

```bash
CRW_INSTALL_DIR=~/.local/bin curl -fsSL https://raw.githubusercontent.com/us/crw/main/install.sh | sh
```

Or use Docker if you prefer containers:

```bash
docker run -p 3000:3000 ghcr.io/us/crw:latest
```

Either option gives you a local endpoint quickly so you can validate real targets before designing a larger deployment.

## What You Get

The self-hosted path is useful when you want to:

- keep target traffic inside your own infrastructure,
- control runtime cost directly,
- and expose scrape, crawl, and map behind your own auth, network, and observability stack.

The API shape stays familiar whether you use the managed cloud or your own deployment.

## Recommended Workflow

1. Boot the service locally or on a small VPS.
2. Validate target URLs with the `scrape`, `map`, and `crawl` routes.
3. Add LightPanda only when your workload requires browser-backed rendering.
4. Put a reverse proxy, auth, and rate limits in front of it before exposing it beyond a trusted environment.

## Early Validation Checklist

Before calling the deployment production-ready, test:

- a simple static page through `scrape`,
- a JS-heavy page with `renderJs: true`,
- a small `map` request,
- a bounded `crawl` request,
- and failure cases such as invalid selectors or target-side 403 responses.

That gives you a much clearer operational picture than only testing a single happy-path URL.

## Example Deployment Pattern

A practical first production shape is:

1. run CRW behind a reverse proxy,
2. keep the API private to your own network or VPN,
3. enable browser rendering only when targets actually need it,
4. add auth and rate limits at the edge,
5. then roll a small real workload through the service before broader adoption.

That is enough for many teams. You do not need a large crawler platform on day one just to validate whether the product fits your workload.

## What This Setup Is Good At

This setup is a good fit when you want to keep traffic inside your own infrastructure, control costs closely, or ship a private scraping service without managing a large crawler platform. If you need public, managed capacity instead, use the hosted product (Cloud only -- fastcrw.com) and keep the same API shape.

## When Not To Self-Host

Choose the managed product instead if:

- you want immediate capacity without operating the service,
- your team does not want to manage browser dependencies,
- or the main goal is product velocity rather than infrastructure control.

Self-hosting is valuable when ownership matters. It is not automatically the best default for every team.

## Common Mistakes

- Exposing the service publicly before adding auth, TLS, and external rate limits.
- Enabling browser-backed rendering for every target instead of only the JS-heavy ones that need it.
- Declaring success after one happy-path scrape instead of testing crawl, map, and failure behavior too.

If you are continuing toward production, read [self-hosting hardening](/docs/self-hosting-hardening) next and keep [credit costs](/docs/credit-costs) nearby for workload sizing.
