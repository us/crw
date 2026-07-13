# Installation — Self-Hosted Web Scraper

## Cloud (no installation needed)

Sign up at [fastcrw.com](https://fastcrw.com) and start using the API immediately.
Native `/v1` endpoints for new CRW integrations, with `/firecrawl/v2` compatibility available for Firecrawl migrations.

```bash
curl -X POST https://api.fastcrw.com/v1/scrape \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'
```

## One-Line Install (Recommended)

The install script auto-detects your OS and architecture, downloads the latest binary, and installs it:

```bash
curl -fsSL https://fastcrw.com/install | sh
```

Or with `wget`:

```bash
wget -qO- https://fastcrw.com/install | sh
```

**Options:**

```bash
# Install to a custom directory:
curl -fsSL https://fastcrw.com/install | CRW_INSTALL_DIR=~/.local/bin sh
```

Supported platforms: macOS (Intel & Apple Silicon), Linux (x64 & ARM64), Windows (via MSYS2/Git Bash).

## Homebrew (macOS & Linux)

```bash
brew install us/crw/crw
```

The tap is added automatically by the fully-qualified name, so no `brew tap` step is needed.
The REST API server and the MCP server are separate formulae:

```bash
brew install us/crw/crw-server   # REST API server
brew install us/crw/crw-mcp      # MCP server
```

Upgrade with `brew upgrade crw`.

## APT (Debian & Ubuntu)

```bash
curl -fsSL https://apt.fastcrw.com/setup.sh | sudo sh
```

This adds the signing key and the CRW repository, then installs `crw`. From then on, upgrades
arrive with the rest of the system through `apt upgrade`.

`crw-server` and `crw-mcp` are in the same repository:

```bash
sudo apt install crw-server
sudo apt install crw-mcp
```

Packages are published for `amd64` and `arm64`. If you would rather add the repository by hand:

```bash
curl -fsSL https://apt.fastcrw.com/gpg.key | sudo gpg --dearmor -o /usr/share/keyrings/crw.gpg
echo "deb [signed-by=/usr/share/keyrings/crw.gpg] https://apt.fastcrw.com stable main" \
  | sudo tee /etc/apt/sources.list.d/crw.list
sudo apt update && sudo apt install crw
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

# MCP stdio binary (full embedded — ~17 MB, includes headless browser engine)
cargo build --release --bin crw-mcp

# MCP stdio binary — lean browser-free proxy (~4.2 MB, no embedded browser)
cargo build --profile release-small --no-default-features -p crw-mcp
```

Binaries are placed in `target/release/` (or `target/release-small/` for the lean build).

The lean build (`--no-default-features`) omits the `embedded` cargo feature that gates the headless browser engine. Use it when you want a minimal proxy-only binary that forwards requests to a remote CRW server or fastcrw.com.

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
