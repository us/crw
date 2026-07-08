<p align="center">
  <img src="docs/logo-animation.gif" alt="fastCRW" width="220" />
</p>

<h1 align="center">fastCRW</h1>

<p align="center">
  The web data API for AI agents — search, scrape, map, and crawl any site into
  clean <strong>markdown</strong> or <strong>JSON</strong>.
</p>

<p align="center">
  Use the managed cloud for zero infra, or self-host the same open-source engine.
  Start with Python, cURL, or MCP — you never touch Rust.
</p>

<p align="center">
  <a href="https://fastcrw.com/register"><img src="https://img.shields.io/badge/Start%20free-500%20credits%2C%20no%20card-7c3aed?style=for-the-badge" alt="Start free"></a>
  &nbsp;
  <a href="https://docs.fastcrw.com"><img src="https://img.shields.io/badge/Docs-docs.fastcrw.com-24292f?style=for-the-badge" alt="Docs"></a>
</p>

<p align="center">
  <a href="https://crates.io/crates/crw-server"><img src="https://img.shields.io/crates/v/crw-server.svg" alt="crates.io"></a>
  <a href="https://pypi.org/project/crw/"><img src="https://img.shields.io/pypi/v/crw.svg?label=pypi" alt="PyPI"></a>
  <a href="https://www.npmjs.com/package/crw-mcp"><img src="https://img.shields.io/npm/v/crw-mcp.svg?label=npm%20mcp" alt="npm crw-mcp"></a>
  <a href="https://github.com/us/crw/actions/workflows/ci.yml"><img src="https://github.com/us/crw/actions/workflows/ci.yml/badge.svg?branch=main&event=push" alt="CI"></a>
  <a href="https://www.bestpractices.dev/projects/13533"><img src="https://www.bestpractices.dev/projects/13533/badge" alt="OpenSSF Best Practices"></a>
  <a href="https://scorecard.dev/viewer/?uri=github.com/us/crw"><img src="https://api.scorecard.dev/projects/github.com/us/crw/badge" alt="OpenSSF Scorecard"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-AGPL--3.0-blue.svg" alt="License"></a>
  <a href="https://github.com/us/crw/stargazers"><img src="https://img.shields.io/github/stars/us/crw?style=social" alt="GitHub Stars"></a>
</p>

<p align="center">
  Works with
  <a href="https://docs.fastcrw.com/mcp-clients/#claude-code">Claude Code</a> ·
  <a href="https://docs.fastcrw.com/mcp-clients/#cursor">Cursor</a> ·
  <a href="https://docs.fastcrw.com/mcp-clients/#windsurf">Windsurf</a> ·
  <a href="https://docs.fastcrw.com/mcp-clients/#cline">Cline</a> ·
  <a href="https://docs.fastcrw.com/mcp-clients/#openai-codex-cli">Codex</a> ·
  <a href="https://docs.fastcrw.com/mcp-clients/#gemini-cli">Gemini CLI</a>
</p>

---

## What you get

- **Any URL → LLM-ready output.** Clean markdown, HTML, links, or schema-validated JSON — no HTML soup, no boilerplate.
- **One API, six operations.** `search` the web and `scrape` a page, or `map` / `crawl` / `extract` / `monitor` a whole site — [see below](#core-operations).
- **Drop-in Firecrawl compatibility.** Migrate existing Firecrawl code by changing one base URL.
- **Managed or self-hosted, same API.** No exit cost — develop against the free open-source binary, ship to the cloud, or the reverse. Nothing changes but the base URL.

## Quickstart — your first scrape in 30 seconds

**[Get a free API key → fastcrw.com/register](https://fastcrw.com/register)** — 500 credits, no card.

```bash
export CRW_API_KEY="crw_live_..."
```

```bash
# cURL — works anywhere, no SDK
curl -X POST https://api.fastcrw.com/v1/scrape \
  -H "Authorization: Bearer $CRW_API_KEY" -H "Content-Type: application/json" \
  -d '{"url":"https://example.com","formats":["markdown"]}'
```

<table>
<tr><th>Python</th><th>Node.js</th></tr>
<tr valign="top"><td>

```bash
pip install crw
```
```python
from crw import CrwClient

crw = CrwClient(api_key="YOUR_API_KEY")
page = crw.scrape("https://example.com",
                  formats=["markdown"])
print(page["markdown"])
```

</td><td>

```bash
npm install crw-sdk
```
```javascript
import { CrwClient } from "crw-sdk";

const crw = new CrwClient({ apiKey: "YOUR_API_KEY" });
const page = await crw.scrape("https://example.com",
                              { formats: ["markdown"] });
console.log(page.markdown);
```

</td></tr>
</table>

In both SDKs `page` is a plain object (`markdown`, `metadata`, `contentType`, …), so `page["markdown"]` / `page.markdown` is clean content:

```markdown
# Example Domain

This domain is for use in documentation examples without needing permission. Avoid use in operations.

[Learn more](https://iana.org/domains/example)
```

Over cURL you get the same fields wrapped in `{"success": true, "data": { … }}`.

Prefer no SDK? Every example works over plain HTTP against `https://api.fastcrw.com`.
Next: [Quickstart docs →](https://docs.fastcrw.com/quickstart/) · [API reference →](https://docs.fastcrw.com/#rest-api)

## Use it in your AI agent (MCP)

fastCRW ships a built-in MCP server, so any MCP host can search/scrape/crawl with no glue code.

```bash
# Claude Code — managed
claude mcp add crw \
  -e CRW_API_URL=https://api.fastcrw.com -e CRW_API_KEY=$CRW_API_KEY \
  -- npx -y crw-mcp

# Claude Code — embedded (no server, no key, runs the engine in-process)
claude mcp add crw -- npx -y crw-mcp
```

Per-client recipes (Cursor, Windsurf, Cline, Continue.dev, Codex, Gemini CLI):
[docs.fastcrw.com/mcp-clients/](https://docs.fastcrw.com/mcp-clients/)

### Agent Skills

Reusable instruction packs that teach coding agents *when* and *how* to use each verb.
Install all 13 into every detected agent with one command:

```bash
npx skills add us/crw            # all skills, every detected agent
npx skills add us/crw@crw-scrape # just one
npx skills add -g us/crw         # global (user-level)
```

`crw` (hub) · `crw-search` · `crw-scrape` · `crw-map` · `crw-crawl` · `crw-parse` ·
`crw-extract` · `crw-watch` · `crw-research` · `crw-dynamic-search` (biggest token-saver) ·
`crw-best-practices` · `crw-migrate` · `crw-self-host`. Full catalog: [`skills/`](./skills/).

## Core operations

| Verb | Endpoint | Does |
|---|---|---|
| **Search** | `POST /v1/search` | Web search (SearXNG), optionally scrape each result |
| **Scrape** | `POST /v1/scrape` | One URL → markdown / HTML / links / schema JSON |
| **Map** | `POST /v1/map` | Discover every URL on a site, fast |
| **Crawl** | `POST /v1/crawl` | Async BFS crawl of a whole site (returns a job id) |
| **Extract** | `POST /v1/scrape` `formats:["json"]` | Structured fields from a JSON Schema |
| **Monitor** | `POST /v1/change-tracking/diff` | Diff a page vs a snapshot — the change-tracking primitive behind scheduled [monitoring](https://docs.fastcrw.com/monitoring/) |

Full reference: [docs.fastcrw.com/#rest-api](https://docs.fastcrw.com/#rest-api).

## SDKs & integrations

```bash
pip install crw          # Python 3.10+
npm install crw-sdk      # TypeScript / Node.js
```

```python
from crw import CrwClient

client = CrwClient(api_key="YOUR_API_KEY")   # or CrwClient() for local embedded mode
client.scrape("https://example.com", formats=["markdown", "links"])
# .search() .map() .crawl() .extract() — one method per operation in the table above
```

Framework extras: `pip install crw[crewai]` · `pip install crw[langchain]`.
Works with [CrewAI](https://pypi.org/project/crw/) · [LangChain](https://pypi.org/project/crw/) ·
[Agno](https://github.com/agno-agi/agno/pull/7183) · [Dify](https://github.com/langgenius/dify) ·
[n8n](https://fastcrw.com/blog/n8n-web-scraping-crw) · [Flowise](https://github.com/FlowiseAI/Flowise/pull/6066).
[All integrations →](https://docs.fastcrw.com/integrations/) · [SDK examples →](https://docs.fastcrw.com/sdk-examples/)

## Managed cloud vs self-host

Same binary, same API in both modes — pick a lane, switch anytime by changing the base URL.

| | **Managed — `api.fastcrw.com`** | **Self-host (free)** |
|---|---|---|
| Best when | You want zero infra, a global proxy network, a dashboard, and usage metering | You want full data residency, your own proxy strategy, and AGPL is fine |
| Start | [Sign up](https://fastcrw.com/register) — 500 free credits, no card | `docker run -p 3000:3000 ghcr.io/us/crw` |
| Search | Managed backend | Bundled SearXNG sidecar |
| Cost | Free tier, then paid plans from **$11/mo** — [pricing](https://fastcrw.com/pricing) | $0 + your hosting bill |
| License | AGPL carve-out for closed-source product code | AGPL-3.0 applies if you expose the API to third parties |

### Self-host in one command

```bash
docker run -p 3000:3000 ghcr.io/us/crw
curl http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{"url":"https://example.com"}'
```

Prefer CLI, Homebrew, Cargo, APT, or Docker Compose with a stealth tier? All install paths and
production hardening: [docs.fastcrw.com/installation/](https://docs.fastcrw.com/installation/) ·
[self-hosting guide →](https://docs.fastcrw.com/#self-hosting)

## Why it's fast (built in Rust)

You don't need Rust to use fastCRW — it's why the numbers below are what they are.
The engine is a single static binary: no Redis, no Node runtime, no Python venv, no
headless-browser sidecar parked in the request path. Cold start is sub-second and idle
RAM sits around **~50 MB**, so one process saturates a $5 VPS instead of a multi-container
stack. An agent that fires N scrapes per task pays the network floor N times — fastCRW
strips process-spawn, JIT-warmup, and browser-navigation overhead out of every one.

## Benchmark

On Firecrawl's own public 1,000-URL dataset, fastCRW leads on **truth-recall (63.74%)**,
**median latency (1914 ms)**, and **p90 tail (4348 ms)** with **0 thrown errors** across 3,000
requests — and recovers 34 URLs the other two miss. Reproducible, not marketing math.

Full numbers, comparison table, and one-command repro: **[BENCHMARKS.md](BENCHMARKS.md)** ·
[fastcrw.com/benchmarks](https://fastcrw.com/benchmarks).

## Migrating from Firecrawl

New projects use native `/v1`. Existing Firecrawl v2 SDK code works against the
`/firecrawl/v2/*` compatibility layer — often just a base-URL swap:

```python
from firecrawl import FirecrawlApp
app = FirecrawlApp(api_url="https://api.fastcrw.com", api_key="YOUR_CRW_API_KEY")
```

Compatibility reduces migration work, not every behavioral difference — check request
bodies, response fields, and unsupported features before moving production traffic.
Field-by-field diff: [`COMPATIBILITY-firecrawl.md`](COMPATIBILITY-firecrawl.md).

## Security

SSRF protection (blocks loopback, private IPs, cloud metadata, non-HTTP schemes), optional
constant-time Bearer auth, RFC 9309 robots.txt, token-bucket rate limiting, and resource caps
(1 MB body, depth 10, 1,000 pages). [Hardening guide →](https://docs.fastcrw.com/self-hosting-hardening/)

## Contributing

Issues and PRs welcome. `make hooks` installs the pre-commit hook; `make check` runs the same
checks as CI. Setup, architecture, and crate layout: **[CONTRIBUTING.md](CONTRIBUTING.md)**.

## License

Open source under [AGPL-3.0](LICENSE). Embedding fastCRW in a closed-source product or
exposing it as a hosted service without meeting AGPL's source-availability terms? The
managed offering at [fastcrw.com](https://fastcrw.com) includes a commercial carve-out,
and standalone commercial licenses are available — **hello@fastcrw.com**.

## Links

[Docs](https://docs.fastcrw.com) ·
[API reference](https://docs.fastcrw.com/#rest-api) ·
[MCP setup](https://docs.fastcrw.com/mcp-clients/) ·
[Benchmarks](https://fastcrw.com/benchmarks) ·
[Pricing](https://fastcrw.com/pricing) ·
[Changelog](CHANGELOG.md) ·
[Discord](https://discord.gg/kkFh2SC8) ·
[X](https://x.com/fast_crw)

<a href="https://www.star-history.com/?repos=us%2Fcrw&type=timeline&legend=bottom-right">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=us/crw&type=timeline&theme=dark&legend=bottom-right" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=us/crw&type=timeline&legend=bottom-right" width="70%" />
 </picture>
</a>

---

<sub>**It is the sole responsibility of end users to respect websites' policies when
scraping.** By default, fastCRW respects `robots.txt` directives.</sub>
