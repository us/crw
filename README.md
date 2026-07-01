<p align="center">
  <img src="docs/logo-animation.gif" alt="fastCRW" width="220" />
</p>

<h1 align="center">fastCRW</h1>

<p align="center">
  Self-hosted, Rust-native web crawler &amp; scraper for AI agents
</p>

The open-source alternative to Firecrawl. One static binary, ~50 MB RAM idle,
a native fastCRW REST API under **`/v1/*`** (scrape, crawl, map, search,
structured extraction, and change tracking), plus a **`/v2/*` Firecrawl
compatibility layer** for migrations, batch scrape, and PDF parse. Self-host free under
AGPL-3.0, or hit our managed API at `api.fastcrw.com`. Reproducible 63.74%
truth-recall on the public 1,000-URL dataset (`diagnose_3way.py`,
2026-05-08) — see [fastcrw.com/benchmarks](https://fastcrw.com/benchmarks).
Built in Rust because every millisecond of agent latency compounds.

<p align="center">
  <a href="https://crates.io/crates/crw-server"><img src="https://img.shields.io/crates/v/crw-server.svg" alt="crates.io"></a>
  <a href="https://github.com/us/crw/actions/workflows/ci.yml"><img src="https://github.com/us/crw/actions/workflows/ci.yml/badge.svg?branch=main&event=push" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-AGPL--3.0-blue.svg" alt="License"></a>
  <a href="https://github.com/us/crw/stargazers"><img src="https://img.shields.io/github/stars/us/crw?style=social" alt="GitHub Stars"></a>
  <a href="https://fastcrw.com"><img src="https://img.shields.io/badge/Managed%20Cloud-fastcrw.com-blueviolet" alt="fastcrw.com"></a>
</p>

<p align="center">
  <a href="https://crates.io/crates/crw-server"><img src="https://img.shields.io/crates/d/crw-server.svg?label=crates.io%20downloads" alt="crates.io downloads"></a>
  <a href="https://www.npmjs.com/package/crw-sdk"><img src="https://img.shields.io/npm/dm/crw-sdk.svg?label=npm%20sdk" alt="npm sdk downloads"></a>
  <a href="https://www.npmjs.com/package/crw-mcp"><img src="https://img.shields.io/npm/dm/crw-mcp.svg?label=npm%20mcp" alt="npm mcp downloads"></a>
  <a href="https://pepy.tech/project/crw"><img src="https://static.pepy.tech/personalized-badge/crw?period=month&units=international_system&left_color=grey&right_color=blue&left_text=pypi%20downloads" alt="PyPI downloads"></a>
  <a href="https://github.com/us/crw/releases"><img src="https://img.shields.io/github/downloads/us/crw/total?label=binary%20downloads" alt="binary downloads"></a>
</p>

<p align="center">
  <a href="https://x.com/fast_crw"><img src="https://img.shields.io/badge/Follow%20on%20X-000000?style=for-the-badge&logo=x&logoColor=white" alt="Follow on X" /></a>
  <a href="https://www.linkedin.com/company/fastcrw"><img src="https://img.shields.io/badge/Follow%20on%20LinkedIn-0077B5?style=for-the-badge&logo=linkedin&logoColor=white" alt="Follow on LinkedIn" /></a>
  <a href="https://discord.gg/kkFh2SC8"><img src="https://img.shields.io/badge/Join%20our%20Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white" alt="Join our Discord" /></a>
</p>

Works with: [Claude Code](https://docs.fastcrw.com/mcp-clients/#claude-code) · [Cursor](https://docs.fastcrw.com/mcp-clients/#cursor) · [Windsurf](https://docs.fastcrw.com/mcp-clients/#windsurf) · [Cline](https://docs.fastcrw.com/mcp-clients/#cline) · [Copilot](https://docs.fastcrw.com/mcp-clients/#any-mcp-client) · [Continue.dev](https://docs.fastcrw.com/mcp-clients/#continue) · [Codex](https://docs.fastcrw.com/mcp-clients/#openai-codex-cli) · [Gemini CLI](https://docs.fastcrw.com/mcp-clients/#gemini-cli)

---

<p align="center">
  <img src=".github/benchmarks/bench-dashboard.png" alt="fastCRW vs Crawl4AI vs Firecrawl on Firecrawl's public 1,000-URL dataset — fastCRW leads on truth-recall, p50, and fast-mode p90 latency" width="100%">
</p>

<p align="center"><sub>Firecrawl's own 1,000-URL public dataset (<code>diagnose_3way.py</code>) — fastCRW leads on truth-recall, median latency, and fast-mode p90. <a href="#benchmark">Full numbers and one-command repro ↓</a></sub></p>

## Why fastCRW?

- **Rust-native, single static binary** — no Redis, no Node.js, no Python venv, no headless-browser sidecar in the request path. One binary, one config file, one process.
- **~50 MB RAM idle** — leaves headroom on a $5 VPS. Browser-render-first stacks (Firecrawl, Crawl4AI) carry a Chromium heap baseline measured in hundreds of MB before a single request lands.
- **Native `/v1`, compatibility `/v2`** — new fastCRW projects should use `/v1/scrape`, `/v1/crawl`, `/v1/map`, and `/v1/search`. Existing Firecrawl v2 SDK projects can use the `/v2/*` compatibility layer (`FirecrawlApp(api_url="https://api.fastcrw.com")`) and validate the documented differences before switching production traffic.
- **Change tracking & monitoring** — diff a page against a prior snapshot (markdown git-diff, per-field JSON, or both) with an optional LLM "meaningful-change" judge. Stateless `changeTracking` primitive in the engine; scheduled monitors + signed-webhook/email alerts on the managed platform. See the [Monitoring docs](https://us.github.io/crw/monitoring).
- **AGPL-3.0 open core + managed option** — self-host free, or point at `api.fastcrw.com` for managed proxy network, dashboard, and SLA without the AGPL obligations on your application code.

---

## Comparison Table

Qualitative positioning vs. the two most-cited alternatives — the same two in
the reproducible benchmark below. Numerical claims trace to the inline sources
noted; everything else is descriptive.

| | **fastCRW** | Firecrawl | Crawl4AI |
|---|---|---|---|
| Language | Rust | Node.js + Playwright | Python + Playwright |
| License | AGPL-3.0 (commercial avail.) | AGPL-3.0 (commercial avail.) | Apache-2.0 |
| Self-host install size | Single static binary (~8 MB) | Multi-container (~500 MB+ image) | ~2 GB image (browser bundled) |
| Memory baseline (idle) | ~50 MB | Large (Chromium heap) | Large (Chromium heap) |
| Firecrawl migration | Yes — `/v2/*` compatibility layer; `/v1/*` is native fastCRW | Native | No |
| MCP server | Built-in (`crw-mcp`) | Separate package | Community add-on |
| Hosted option | `api.fastcrw.com` (BYOK or managed) | firecrawl.dev | None official |
| Reproducible public benchmark | Yes — 63.74% truth-recall on 1,000-URL dataset (`diagnose_3way.py`, 2026-05-08) | Vendor-published only | Vendor-published only |

Pricing/spec cells where claimed link to the vendor page; everything else
is the qualitative architectural shape, not a comparison number.

---

## Quickstart

Hit the managed API at `api.fastcrw.com`, or self-host the same binary.

```bash
# /v1/scrape — URL → markdown / HTML / JSON / links
curl -X POST https://api.fastcrw.com/v1/scrape \
  -H "Authorization: Bearer $CRW_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"url":"https://example.com","formats":["markdown"]}'
```

```bash
# /v1/scrape + formats:["json"] — structured JSON extraction via a JSON Schema
curl -X POST https://api.fastcrw.com/v1/scrape \
  -H "Authorization: Bearer $CRW_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "url":"https://example.com",
    "formats":["json"],
    "jsonSchema":{
      "type":"object",
      "properties":{"title":{"type":"string"}}
    }
  }'
```

```bash
# /v1/crawl — async multi-page job (returns a job id; poll with /v1/crawl/:id)
curl -X POST https://api.fastcrw.com/v1/crawl \
  -H "Authorization: Bearer $CRW_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"url":"https://docs.example.com","maxDepth":2,"maxPages":50}'
```

```bash
# Self-host (no auth, localhost) — single docker command
docker run -p 3000:3000 ghcr.io/us/crw
curl http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{"url":"https://example.com"}'
```

Other install paths (each documented under
[`Install`](#install) further down):

```bash
npx crw-mcp                           # zero install — runs the embedded engine
pip install crw                        # Python SDK (auto-downloads binary)
brew install us/crw/crw                # Homebrew
cargo install crw-cli                  # Cargo
curl -fsSL https://fastcrw.com/install | sh
```

---

## Why Rust?

Cold start is sub-second and the resident memory ceiling is bounded by the
crawl queue, not by a JavaScript runtime or a headless browser parked in
the background. An agent that issues N scrapes per task pays the network
floor N times — anything you add on top (process spawn, JIT warmup,
browser navigation overhead) multiplies. Pushing the request-path
language down to Rust strips that surcharge out of every call. The same
property lets one static binary saturate a $5 VPS instead of needing a
multi-container compose stack, which is why the idle footprint is in the
tens of MB rather than the hundreds.

---

## MCP + SDK quickstart

fastCRW ships a built-in MCP server so any MCP-compatible agent (Claude
Code, Cursor, Windsurf, Cline, Continue.dev, Codex, Gemini CLI) can call
scraping tools without bespoke glue. Embedded mode runs the engine
in-process — no server, no API key, no setup. The `crw` Python SDK and
the `crw-mcp` Node binary both shell to the same Rust core.

```bash
npm install -g crw-mcp          # MCP server (Node wrapper)
pip install crw                 # Python SDK (auto-downloads binary)
claude mcp add crw -- npx -y crw-mcp                                       # Claude Code, embedded
claude mcp add crw \
  -e CRW_API_URL=https://api.fastcrw.com -e CRW_API_KEY=… \
  -- npx -y crw-mcp                                                        # Claude Code, managed
```

Per-client config recipes (Claude Desktop, Cursor, Windsurf, Cline,
Continue.dev) live under [docs.fastcrw.com/mcp-clients/](https://docs.fastcrw.com/mcp-clients/).

---

## Agent Skills

Beyond raw MCP tools, fastCRW ships a set of [agent skills](https://www.skills.sh) —
reusable instruction packs that teach AI coding agents *when* and *how* to scrape,
crawl, map, search, parse, extract, and change-track the web. Install into any
agent (Claude Code, Codex, Cursor, OpenCode, Gemini CLI, Windsurf + more) with one
command:

```bash
npx skills add us/crw                 # all 12 skills, into every detected agent
npx skills add us/crw@crw-scrape      # just one
npx skills add -g us/crw              # global (user-level)
```

| Tier | Skills |
|------|--------|
| **Core / verb ladder** | `crw` (hub) · `crw-search` · `crw-scrape` · `crw-map` · `crw-crawl` · `crw-parse` · `crw-extract` · `crw-watch` |
| **Quality / meta** | `crw-dynamic-search` (context-isolating filter — the biggest token-saver) · `crw-best-practices` |
| **Migration / ops** | `crw-migrate` (one-line Firecrawl `base_url` swap) · `crw-self-host` |

The skills drive the `crw` CLI, the `crw-mcp` tools, or the REST API — pick whatever
surface you have. No API key needed for self-hosted search (SearXNG). Full catalog
and per-skill docs: [`skills/`](./skills/). Also packaged as a plugin marketplace
for Claude Code, Codex, and Cursor (`.claude-plugin/`, `.codex-plugin/`, `.cursor-plugin/`).

---

## Self-host vs Managed

| | **Self-host (free)** | **Managed — `api.fastcrw.com`** |
|---|---|---|
| Best when | You want full data residency, AGPL is fine, you can run your own proxy strategy, latency to your infra matters more than ours. | You want zero infra, a global proxy network, a dashboard, usage metering, and AGPL carve-out for closed-source product code. |
| Install | `docker run -p 3000:3000 ghcr.io/us/crw` or `cargo install crw-server`. | Sign up at [fastcrw.com](https://fastcrw.com) — 500 free credits, no card. |
| Search | Bundled SearXNG sidecar (`docker compose up`). | Managed search backend. |
| Proxy rotation | Bring your own pool (`proxy_list` + `proxy_rotation`: round_robin / random / sticky_per_host) — rotated across the HTTP **and** JS/Chrome paths for scrape, crawl, and map; per-request BYOP supported. LightPanda can't proxy, so it's skipped (fail-closed) when a proxy is active. | Managed proxy network. |
| Cost | $0 + your hosting bill. | From $13/mo; pricing on [fastcrw.com/pricing](https://fastcrw.com/pricing). |
| License obligations | AGPL-3.0 applies if you expose the API to third parties. | AGPL carve-out included. |

The binary is the same in both modes — you can develop against your
self-hosted instance and ship to managed without code changes.

---

## Install

### MCP server (`crw-mcp`) — recommended for AI agents

```bash
npx crw-mcp                              # zero install (npm)
pip install crw                          # Python SDK (auto-downloads binary)
brew install us/crw/crw-mcp              # Homebrew
cargo install crw-mcp                    # Cargo (full embedded, ~17 MB)
docker run -i ghcr.io/us/crw crw-mcp     # Docker
```

**Lean browser-free proxy build** (~4.2 MB, no headless browser engine — proxy/cloud mode only):

```bash
cargo build --profile release-small --no-default-features -p crw-mcp
```

### CLI (`crw`) — scrape URLs from your terminal

```bash
brew install us/crw/crw

# One-line install (auto-detects OS & arch):
curl -fsSL https://fastcrw.com/install | CRW_BINARY=crw sh

# APT (Debian/Ubuntu):
curl -fsSL https://apt.fastcrw.com/gpg.key | sudo gpg --dearmor -o /usr/share/keyrings/crw.gpg
echo "deb [signed-by=/usr/share/keyrings/crw.gpg] https://apt.fastcrw.com stable main" \
  | sudo tee /etc/apt/sources.list.d/crw.list
sudo apt update && sudo apt install crw

cargo install crw-cli
```

### API server (`crw-server`) — native REST API plus Firecrawl compatibility

For serving multiple apps, other languages (Node.js, Go, Java), or as a
shared microservice.

```bash
brew install us/crw/crw-server

# One-line install:
curl -fsSL https://fastcrw.com/install | CRW_BINARY=crw-server sh

# Docker:
docker run -p 3000:3000 ghcr.io/us/crw
```

Docker Compose ships with `lightpanda` by default; `chrome` is opt-in:

```bash
docker compose up -d                                         # http + lightpanda
docker compose --profile heavy up -d                         # + chrome failover
docker compose -f docker-compose.yml \
  -f docker-compose.stealth.yml --profile stealth up -d      # browserless stealth tier
```

There's also an **optional [Camoufox](https://github.com/daijro/camoufox)
stealth tier** (REST sidecar, opt-in) for fingerprint-blocked targets the CDP
renderers can't pass — off by default and never touches the `auto` chain unless
you turn it on. Build with `--features camoufox`; see [JS rendering →
Camoufox](https://docs.fastcrw.com/#js-rendering).

See the [self-hosting guide](https://docs.fastcrw.com/#self-hosting) for
production hardening, auth, reverse proxy, and resource tuning.

---

## API endpoints

| Method | Endpoint | Description |
|---|---|---|
| `POST` | `/v1/scrape` | Scrape a single URL, optionally with LLM extraction or summary |
| `POST` | `/v1/crawl` | Start async BFS crawl (returns job ID) |
| `GET` | `/v1/crawl/:id` | Check crawl status and retrieve results |
| `DELETE` | `/v1/crawl/:id` | Cancel a running crawl job |
| `POST` | `/v1/map` | Discover all URLs on a site |
| `POST` | `/v1/search` | Web search via SearXNG sidecar, with optional content scraping |
| `POST` | `/v1/change-tracking/diff` | Diff a scrape against a supplied snapshot (the [monitoring](https://us.github.io/crw/monitoring) primitive) — single or batch |
| `GET` | `/health` | Health check (no auth required) |
| `POST` | `/mcp` | Streamable HTTP MCP transport |

**Firecrawl v2 compatibility surface** — `scrape`, `crawl`, `map`, and `search` are also served under `/v2/*` with Firecrawl v2 request/response shapes, plus compatibility-only `POST /v2/batch/scrape`, `POST /v2/parse` (PDF → markdown), and `GET /v2/crawl/active`. Use this when migrating existing Firecrawl v2 SDK code; new fastCRW integrations should start with `/v1`.

Full reference at [docs.fastcrw.com/#rest-api](https://docs.fastcrw.com/#rest-api).
The Firecrawl compatibility matrix (field-by-field diff) lives in
[`COMPATIBILITY-firecrawl.md`](COMPATIBILITY-firecrawl.md).

---

## Benchmark

Reproduce it yourself first — the canonical harness is `diagnose_3way.py`
(matches truth text against `md + strip_md_links(md)`, applied identically
to all three tools — a fairness control, not a looser number):

```bash
cd ~/coding/crw/crw-opencore
docker compose -f docker-compose.yml -f docker-compose.override.yml \
               -f docker-compose.stealth.yml --profile stealth up -d
docker start crawl4ai-bench
cd ~/coding/crw/competitors/firecrawl && docker compose up -d

cd ~/coding/crw/crw-opencore
uv run python bench/diagnose_3way.py \
  --max-urls 1000 --tools crw,crawl4ai,firecrawl \
  --concurrency 5 --timeout 120 \
  --out bench/server-runs/diag3w-1000-full.jsonl
```

3-way scrape benchmark, full 1,000-URL run on
[Firecrawl's `scrape-content-dataset-v1`](https://huggingface.co/datasets/firecrawl/scrape-content-dataset-v1)
(`diagnose_3way.py`, 2026-05-08, concurrency 5, timeout 120s):

| Metric | fastCRW | crawl4ai | Firecrawl |
|---|---|---|---|
| **Truth-recall (522/819 labeled URLs)** <sup>recall mode</sup> | **63.74%** | 59.95% | 56.04% |
| **p50 latency** | **1914 ms** | 1916 ms | 2305 ms |
| **p90 latency** <sup>fast mode</sup> | **4348 ms** | 4754 ms | 6937 ms |
| Thrown errors (3,000 requests) | 0 | 0 | 0 |

fastCRW leads on every axis — top truth-recall, fastest median, and the
lowest p90 tail — with **0 thrown errors** across all 3,000 requests, and it
uniquely recovers **34 URLs the other two miss** (70% more than crawl4ai and
Firecrawl combined). The 63.74% denominator is 819 labeled/matchable URLs,
not 3,000 requests, not 1,000.

**Two tunable modes, one engine, one config toggle.** *Recall mode* (the full
ladder) maximizes truth-recall, recovering the long tail of hard pages the
others miss. *Fast mode* (LightPanda-only, no Chrome tier) optimizes the
latency tail — **p90 4348 ms, the lowest of the three** (`diagnose_3way.py`,
N=1000). Same binary, same API; pick accuracy or latency per workload.

Full result of record:
[`bench/server-runs/RESULT_3WAY_1000_FULL.md`](bench/server-runs/RESULT_3WAY_1000_FULL.md).

---

## SDKs and integrations

### Python

```bash
pip install crw
```

```python
from crw import CrwClient

# Managed (includes web search):
client = CrwClient(api_url="https://api.fastcrw.com", api_key="YOUR_API_KEY")
# Local (embedded, no server needed):
# client = CrwClient()

result = client.scrape("https://example.com", formats=["markdown", "links"])
pages = client.crawl("https://docs.example.com", max_depth=2, max_pages=50)
urls = client.map("https://example.com")
results = client.search("AI news", limit=10, sources=["web", "news"])
```

Requires Python 3.10+. Local mode auto-downloads `crw-mcp` on first use.

Framework extras:

```bash
pip install crw[crewai]    # CRW scraping tools for CrewAI agents
pip install crw[langchain] # CRW document loader for LangChain
```

### TypeScript / Node.js

```bash
npm install crw-sdk
```

[SDK examples →](https://docs.fastcrw.com/sdk-examples/)

### Frameworks & platforms

[CrewAI](https://pypi.org/project/crw/) · [LangChain](https://pypi.org/project/crw/)
· [Agno](https://github.com/agno-agi/agno/pull/7183) · [Dify](https://github.com/langgenius/dify)
· [n8n](https://fastcrw.com/blog/n8n-web-scraping-crw) · [Flowise](https://github.com/FlowiseAI/Flowise/pull/6066)

[All integrations →](https://docs.fastcrw.com/integrations/)

---

## Architecture

```
┌─────────────────────────────────────────────┐
│                 crw-server                  │
│         Axum HTTP API + Auth + MCP          │
├──────────┬──────────┬───────────────────────┤
│ crw-crawl│crw-extract│    crw-renderer      │
│ BFS crawl│ HTML→MD   │  HTTP + CDP(WS)      │
│ robots   │ LLM/JSON  │  LightPanda/Chrome   │
│ sitemap  │ clean/read│  auto-detect SPA     │
├──────────┴──────────┴───────────────────────┤
│                 crw-core                    │
│        Types, Config, Errors                │
└─────────────────────────────────────────────┘
```

| Crate | Description |
|-------|-------------|
| [`crw-core`](crates/crw-core) | Core types, config, and error handling |
| [`crw-renderer`](crates/crw-renderer) | HTTP + CDP browser rendering engine |
| [`crw-extract`](crates/crw-extract) | HTML → markdown/plaintext extraction |
| [`crw-crawl`](crates/crw-crawl) | Async BFS crawler with robots.txt & sitemap |
| [`crw-server`](crates/crw-server) | Axum API server (native `/v1` plus Firecrawl `/v2` compatibility) |
| [`crw-mcp`](crates/crw-mcp) | MCP stdio server (embedded + proxy mode) |
| [`crw-cli`](crates/crw-cli) | Standalone CLI (`crw` binary, no server) |

[Full architecture docs →](https://docs.fastcrw.com/architecture/)

---

## Security

- **SSRF protection** — blocks loopback, private IPs, cloud metadata (`169.254.x.x`), IPv6 mapped addresses, and non-HTTP schemes (`file://`, `data:`)
- **Auth** — optional Bearer token with constant-time comparison
- **robots.txt** — RFC 9309 compliant with wildcard patterns
- **Rate limiting** — token-bucket algorithm, returns 429 with `error_code`
- **Resource limits** — max body 1 MB, max crawl depth 10, max pages 1,000

[Full security docs →](https://docs.fastcrw.com/self-hosting-hardening/)

---

## Contributing

Contributions are welcome — issues and PRs both.

1. Fork the repository
2. Install pre-commit hooks: `make hooks`
3. Create your feature branch (`git checkout -b feat/my-feature`)
4. Commit your changes (`git commit -m 'feat: add my feature'`)
5. Push to the branch (`git push origin feat/my-feature`)
6. Open a Pull Request

The pre-commit hook runs the same checks as CI (`cargo fmt`, `cargo clippy`,
`cargo test`). Run manually with `make check`.

### Contributors

<p>
  <a href="https://github.com/us"><img src="https://github.com/us.png?size=64" width="64" height="64" alt="us"/></a>
  <a href="https://github.com/adambenhassen"><img src="https://github.com/adambenhassen.png?size=64" width="64" height="64" alt="adambenhassen"/></a>
  <a href="https://github.com/mj520"><img src="https://github.com/mj520.png?size=64" width="64" height="64" alt="mj520"/></a>
</p>

---

## License

fastCRW is open source under [AGPL-3.0](LICENSE). If you embed fastCRW in
a closed-source product or expose it as a hosted service to third parties
and you can't comply with AGPL's source-availability requirements, the
managed offering at [fastcrw.com](https://fastcrw.com) includes a
commercial carve-out, and standalone commercial licenses are available
on request — write to **hello@fastcrw.com**.

---

## Links

- **Documentation:** [docs.fastcrw.com](https://docs.fastcrw.com)
- **API reference:** [docs.fastcrw.com/#rest-api](https://docs.fastcrw.com/#rest-api)
- **MCP setup guide:** [docs.fastcrw.com/#mcp](https://docs.fastcrw.com/#mcp)
- **Playground:** [docs.fastcrw.com/playground/](https://docs.fastcrw.com/playground/)
- **Benchmarks:** [fastcrw.com/benchmarks](https://fastcrw.com/benchmarks)
- **Marketing site:** [fastcrw.com](https://fastcrw.com)
- **Changelog:** [`CHANGELOG.md`](CHANGELOG.md)
- **X / Twitter:** [@fast_crw](https://x.com/fast_crw)
- **LinkedIn:** [fastcrw](https://www.linkedin.com/company/fastcrw)
- **Discord:** [discord.gg/kkFh2SC8](https://discord.gg/kkFh2SC8)
- **MCP Registry:** [registry.modelcontextprotocol.io](https://registry.modelcontextprotocol.io/?q=crw)

---

## Star History

<a href="https://www.star-history.com/?repos=us%2Fcrw&type=timeline&legend=bottom-right">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=us/crw&type=timeline&theme=dark&legend=bottom-right" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=us/crw&type=timeline&legend=bottom-right" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=us/crw&type=timeline&legend=bottom-right" />
 </picture>
</a>

---

**It is the sole responsibility of end users to respect websites' policies
when scraping.** Users are advised to adhere to applicable privacy
policies and terms of use. By default, fastCRW respects `robots.txt`
directives.
