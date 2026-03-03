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

## Docker Compose

The included `docker-compose.yml` runs crw with a LightPanda sidecar:

```yaml
services:
  crw:
    image: ghcr.io/us/crw:latest
    ports:
      - "3000:3000"
  lightpanda:
    image: lightpanda/lightpanda:latest
    command: ["serve", "--host", "0.0.0.0", "--port", "9222"]
```
