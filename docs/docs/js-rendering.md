# JS Rendering

crw supports JavaScript rendering for single-page applications (SPAs) and JS-heavy sites via the Chrome DevTools Protocol (CDP).

## Rendering Modes

Set the rendering mode in your config:

```toml
[renderer]
mode = "auto"          # auto | lightpanda | playwright | chrome | none
page_timeout_ms = 30000
pool_size = 4
```

| Mode | Behavior |
|------|----------|
| `auto` | Use HTTP first, detect SPAs, fall back to CDP if needed |
| `lightpanda` | Always use LightPanda for JS rendering |
| `playwright` | Always use Playwright for JS rendering |
| `chrome` | Always use Chrome for JS rendering |
| `none` | HTTP only, never render JS |

## Per-request control

Override the rendering mode per request using `renderJs`:

```json
{
  "url": "https://spa-app.com",
  "renderJs": true,
  "waitFor": 3000
}
```

| Value | Behavior |
|-------|----------|
| `null` (default) | Auto-detect based on heuristics |
| `true` | Force CDP rendering |
| `false` | HTTP only |

## When To Turn It On

Enable JS rendering when the page content is not present in the initial HTML response. Typical examples:

- single-page applications,
- pages that fetch content after hydration,
- and sites where the meaningful body is assembled client-side.

Do not enable it blindly for every request. HTTP-only fetches are faster and cheaper.

## Choosing a `waitFor` Value

Start with the smallest value that works:

- `500` to `1000` for lightly hydrated pages,
- `2000` for typical JS-heavy pages,
- `3000` to `5000` only when you have confirmed the target hydrates slowly.

:::warning
Long waits are not automatically safer. They increase latency and can hide the fact that the page is blocked rather than merely slow.
:::

## Auto-detection

When `renderJs` is `null`, crw fetches the page via HTTP first, then checks for SPA signals:

**Triggered when body text < 200 chars and:**
- Contains `id="root"`, `id="app"`, `id="__next"`, `id="__nuxt"`, `id="__gatsby"`, `id="svelte"`
- Contains `ng-app`, `data-reactroot`
- Contains `window.__initial_state__`, `__next_data__`, `window.__remixcontext`, `window.__astro`

**Triggered when:**
- `<noscript>` contains "enable javascript"
- Body text < 500 chars and URL matches Framer, Webflow, Wix, or Squarespace domains

## CDP Backends

### LightPanda (Recommended)

Fastest option. Lightweight browser engine purpose-built for scraping.

```bash
# Auto-install
crw-server setup

# Manual start
lightpanda serve --host 127.0.0.1 --port 9222 &
```

```toml
[renderer.lightpanda]
ws_url = "ws://127.0.0.1:9222/"
```

### Playwright

```toml
[renderer.playwright]
ws_url = "ws://playwright:9222"
```

### Chrome / Chromium

```toml
[renderer.chrome]
ws_url = "ws://chrome:9222"
```

## How CDP rendering works

1. Open a new browser tab via `Target.createTarget`
2. Navigate to the URL
3. Attach to the target via `Target.attachToTarget`
4. Wait for the configured time (`waitFor` or default 2000ms)
5. Execute `Runtime.evaluate("document.documentElement.outerHTML")` to get rendered HTML
6. Close the tab via `Target.closeTarget`

## Cloud vs Self-hosted

- **Cloud**: JS rendering is always available. The managed infrastructure runs a LightPanda sidecar alongside the engine. Available on [fastcrw.com](https://fastcrw.com) (cloud).
- **Self-hosted**: You must run `crw-server setup` or configure a CDP browser (LightPanda, Chrome, or Playwright) in your `config.toml` under `[renderer]`. If no JS renderer is configured, requests with `renderJs: true` will fall back to HTTP-only fetching and include a warning.

## What To Inspect in the Response

When rendered output looks wrong, check:

- `metadata.renderedWith` to verify a browser was actually used,
- `metadata.elapsedMs` to understand the cost of the request,
- and `warning` to catch anti-bot or fallback situations.

## Troubleshooting

- **Empty content from JS-heavy sites**: Increase `waitFor` (e.g., `3000`-`5000`). Some SPAs need extra time to hydrate.
- **`renderedWith: "http_only_fallback"` in metadata**: JS rendering was requested but no renderer is available. Check your deployment configuration.
- **Internal error on `renderJs: true`**: Verify the LightPanda sidecar is running and reachable. Check `/health` for renderer status.
- **Still poor output after increasing `waitFor`**: The issue may be anti-bot protection or authentication flow, not rendering delay.

## Docker Compose

The included `docker-compose.yml` runs crw with a LightPanda sidecar:

```yaml
services:
  crw:
    image: ghcr.io/us/crw:latest
    ports:
      - "3000:3000"
    environment:
      - CRW_RENDERER__LIGHTPANDA__WS_URL=ws://lightpanda:9222
  lightpanda:
    image: lightpanda/lightpanda:latest
    command: ["serve", "--host", "0.0.0.0", "--port", "9222"]
```
