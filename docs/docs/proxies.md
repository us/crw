# Proxies & Rotation

crw can route requests through your own pool of proxies and **rotate across them**
— so each request egresses from a different IP. This works across the **HTTP fetch
path and the JS/Chrome (CDP) path**, for **scrape, crawl, and map**, on both the
`/v1` and `/v2` APIs, and from the CLI.

There are two ways to use proxies:

1. **Bring your own pool (BYOP)** — give crw a list of proxy URLs and it rotates.
2. **A single upstream proxy** — point crw at one proxy (or a provider's rotating
   gateway that rotates IPs for you).

## Quick start (self-host)

Add a pool to your `config.toml` under `[crawler]`:

```toml
[crawler]
# Rotate across your own proxies, one per request.
proxy_list = [
  "http://user:pass@proxy-a.example:8080",
  "http://user:pass@proxy-b.example:8080",
  "socks5://user:pass@proxy-c.example:1080",
]
proxy_rotation = "sticky_per_host"   # default
```

Or via environment variables (handy for Docker):

```bash
CRW_CRAWLER__PROXY_LIST="http://user:pass@a:8080,http://user:pass@b:8080"
CRW_CRAWLER__PROXY_ROTATION="sticky_per_host"
```

`CRW_CRAWLER__PROXY_LIST` accepts a comma-separated list, a JSON array string, or
a TOML array in the config file.

That's it — every scrape/crawl/map now egresses through the pool.

## Rotation strategies

Set `proxy_rotation` (or per request, see below) to one of:

| Strategy | Behaviour | Use when |
|---|---|---|
| **`sticky_per_host`** (default) | Each target host is pinned to one proxy for the process lifetime (deterministic hash). | **Recommended.** Keeps cookies/TLS sessions coherent per host — anti-bot systems (Cloudflare, DataDome…) flag mid-session IP changes, so a stable IP per host *reduces* blocks while still spreading load across many hosts. |
| `round_robin` | Cycle through the pool, one step per request. | You want even load distribution and your targets don't bind sessions to IPs. |
| `random` | Pick a uniformly random proxy per request. | Simple spread; no ordering guarantees. |

## Per-request (bring your own proxy)

Any scrape/crawl request can carry its own pool, which **takes precedence** over
the server config. Supported on `/v1` and `/v2` (camelCase or snake_case):

```bash
curl -X POST https://your-host/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com",
    "proxy_list": [
      "http://user:pass@a.example:8080",
      "http://user:pass@b.example:8080"
    ],
    "proxy_rotation": "round_robin"
  }'
```

```python
from firecrawl import FirecrawlApp
app = FirecrawlApp(api_url="https://your-host")
app.scrape_url("https://example.com", params={
    "proxyList": ["http://a:8080", "http://b:8080"],
    "proxyRotation": "sticky_per_host",
})
```

Precedence: **per-request `proxy_list` › per-request `proxy` › config `proxy_list`
› config `proxy` › none.**

## Single proxy

If you only have one endpoint (e.g. a provider's rotating-gateway URL that rotates
IPs on their side), use the single `proxy` field:

```toml
[crawler]
# Most residential providers give you ONE gateway URL that auto-rotates per request.
proxy = "http://user:pass@gate.smartproxy.com:7000"
# proxy = "socks5://user:pass@proxy:1080"
```

## Coverage

| Path | Rotates? | Notes |
|---|---|---|
| HTTP fetch (scrape, crawl pages, map) | ✅ | Warm per-proxy connection pool, picked by host. |
| JS / Chrome (CDP) | ✅ | Per-request `Target.createBrowserContext { proxyServer }` — vanilla Chrome. |
| LightPanda | ⛔ skipped | LightPanda has no proxy support; when a proxy is active crw **skips it and uses Chrome** (it never silently bypasses the proxy — see Fail-closed). |
| `map` | config pool only | Map honors the server-config proxy/pool; per-request BYOP on `map` is not supported. |

## SOCKS5 notes

- **HTTP path:** `socks5` / `socks5h`, with or without credentials, fully supported.
- **JS/Chrome path:** Chrome can route through a SOCKS proxy, but **cannot
  authenticate one** (no `Fetch.authRequired` for SOCKS). A `socks5://user:pass@…`
  proxy is rejected on the JS path — use an `http`/`https` proxy for JS rendering,
  or a credential-less SOCKS proxy. (`socks5h` is normalized to `socks5` for Chrome.)

## Fail-closed (no real-IP leaks)

When a proxy is configured, crw never silently falls back to a direct connection:

- A **malformed proxy URL** is a hard error at startup (config), at request time
  (per-request BYOP → `400`), or fails the crawl job — never a silent direct fetch.
- If a request needs JS and the **only** available renderer is LightPanda (which
  can't proxy), the request **errors** rather than leaking your real IP.

This means: if a proxy is set, your real IP is never used for that request.

## Residential / geo-targeted tier (advanced)

Separately from `proxy_list`, crw has an opt-in **residential `chrome_proxy`
renderer tier** for IP-flagged targets, with per-request country selection
(`country` on the scrape body → `__cr.<cc>` for DataImpulse-style providers). See
[JS Rendering](js-rendering.md) for that tier's setup (`proxy_base_user`,
`proxy_base_pass`, `[renderer.chrome_proxy]`).

## Managed cloud

On the managed API (`api.fastcrw.com`) you can bring your own proxy pool on paid
plans, or use the managed proxy network without running any proxies yourself.
Proxied requests carry a small credit surcharge (bandwidth cost). See
[fastcrw.com/pricing](https://fastcrw.com/pricing).

## Verify it works

A ready-made end-to-end check (two local logging proxies + assertions for HTTP
rotation, JS-path egress, and leak-safety) ships with the repo:

```bash
./scripts/verify-proxy-rotation.sh
WITH_CHROME=1 ./scripts/verify-proxy-rotation.sh   # also exercises the JS path
```
