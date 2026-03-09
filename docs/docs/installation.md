# Installation — Self-Hosted Web Scraper

## Cloud (no installation needed)

Sign up at [fastcrw.com](https://fastcrw.com) and start using the API immediately.
Same Firecrawl-compatible endpoints, zero infrastructure.

```bash
curl -X POST https://fastcrw.com/api/v1/scrape \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'
```

## From crates.io

```bash
# CLI tool (no server required)
cargo install crw-cli

# REST API server
cargo install crw-server
```

## From source

```bash
git clone https://github.com/us/crw.git
cd crw

# CLI tool (no server, no setup)
cargo build --release --bin crw

# REST API server (without JS rendering)
cargo build --release --bin crw-server

# REST API server (with CDP/JS rendering)
cargo build --release --bin crw-server --features crw-server/cdp

# MCP stdio binary
cargo build --release --bin crw-mcp
```

Binaries are placed in `target/release/`.

## Docker

```bash
# Pre-built image
docker run -p 3000:3000 ghcr.io/us/crw:latest

# With docker-compose (includes LightPanda sidecar)
docker compose up
```

The Docker image uses a multi-stage build: `rust:1.93-bookworm` for building, `debian:bookworm-slim` for runtime. The compose file includes a LightPanda sidecar for JS rendering.

## JS Rendering Setup

crw supports JavaScript rendering via CDP (Chrome DevTools Protocol). The fastest option is LightPanda:

```bash
# Automatic setup (downloads LightPanda + creates config.local.toml)
crw-server setup

# Start LightPanda
lightpanda serve --host 127.0.0.1 --port 9222 &

# Start crw
crw-server
```

The `setup` command detects your platform (Linux x86_64, macOS aarch64) and downloads the appropriate LightPanda binary to `~/.local/bin/lightpanda`.

### Other CDP backends

You can also use Playwright or Chrome. Set the WebSocket URL in your config:

```toml
[renderer]
mode = "auto"

[renderer.playwright]
ws_url = "ws://playwright:9222"

# or
[renderer.chrome]
ws_url = "ws://chrome:9222"
```

## Verify

```bash
crw-server
# Server starts on http://localhost:3000

curl http://localhost:3000/health
```
