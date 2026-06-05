# Docker

Run crw with Docker Compose for the easiest setup with JS rendering included.

## Pre-built Image

```bash
docker pull ghcr.io/us/crw:latest
docker run -p 3000:3000 ghcr.io/us/crw:latest
```

Available tags: `latest`, `0.0.1`, `0.0`

## Docker Compose

```bash
git clone https://github.com/us/crw.git
cd crw
docker compose up
```

> **Build memory.** The `crw` image builds from source with a release LTO link
> (`-C lto -C codegen-units=1` in the release profile), which is memory-hungry.
> Give Docker **≥ 8 GB RAM** for the build — on a 4 GB Docker VM the final
> `crw-server` link gets OOM-killed (`failed to solve: ResourceExhausted: cannot
> allocate memory`). If you're capped at 4 GB, serialize the build so only one
> codegen unit links at a time: `CARGO_BUILD_JOBS=1 docker compose build crw`,
> then `docker compose up -d`. This is a one-time build cost — the runtime image
> is small.

The bundled `docker-compose.yml` starts these services:

| Service | Port | Default? | Description |
|---------|------|----------|-------------|
| **crw** | 3000 | ✅ | API server (loads `config.docker.toml`) |
| **searxng** | 8080 | ✅ | SearXNG meta-search backend for `/v1/search` |
| **lightpanda** | 9222 | ✅ | Lightweight headless browser for JS rendering |
| **chrome** | 9222 | `--profile heavy` | Full Chromium fallback for complex SPAs |
| **chrome-stealth** | 3000 | `--profile stealth` | Anti-fingerprint Chromium (browserless, SSPL-licensed) |

The `crw` service reads its configuration from the mounted `config.docker.toml` (via
`CRW_CONFIG=config.docker`), which already points each renderer and the search backend at the matching
service name on Compose's default bridge network (`lightpanda:9222`, `chrome:9222`, `searxng:8080`). You
don't need to wire renderer URLs through environment variables — they're in the config file.

The optional `chrome` / `chrome-stealth` tiers are opt-in so small hosts skip the ~500 MB Chromium image:

```bash
docker compose --profile heavy up -d      # add the vanilla Chromium fallback
docker compose --profile stealth up -d    # add the anti-fingerprint tier (review the SSPL license first)
```

## Search (SearXNG)

`/v1/search` (and the `crw_search` MCP tool) is backed by the bundled **searxng** service, reachable inside
the Compose network as `searxng:8080`. This is configured by default in `config.docker.toml`:

```toml
[search]
searxng_url = "http://searxng:8080"
```

To point CRW at an **external** SearXNG instead of the sidecar, override it without editing the file:

```yaml
services:
  crw:
    environment:
      - CRW_SEARCH__SEARXNG_URL=http://your-searxng-host:8080   # env wins over the config file
```

> **Two different URLs — don't confuse them.** `SEARXNG_BASE_URL` (set on the `searxng` service) is
> SearXNG's *own* self-reference for the links it renders. `[search].searxng_url` /
> `CRW_SEARCH__SEARXNG_URL` is the host **CRW** calls. They happen to share the value `searxng:8080` in the
> bundled stack, but they serve different roles.

> **Cold start.** `crw` waits for `searxng` to report healthy (`depends_on: condition: service_healthy`),
> so the first search after `docker compose up` can take ~15–30 s once the images are present (longer on the
> first pull). A `target_unreachable` or `timeout` error in the first few seconds usually just means SearXNG
> hasn't finished booting yet — the server logs the configured search host at startup so you can confirm it.

## Dockerfile

Multi-stage build for minimal image size:

```dockerfile
FROM rust:1.93-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release --bin crw-server --features crw-server/cdp

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/crw-server /usr/local/bin/crw-server
COPY config.default.toml /app/config.default.toml
WORKDIR /app
EXPOSE 3000
CMD ["crw-server"]
```

## Custom Configuration

Override settings via environment variables:

```yaml
services:
  crw:
    build: .
    ports:
      - "8080:8080"
    environment:
      - RUST_LOG=debug
      - CRW_SERVER__PORT=8080
      - CRW_CRAWLER__REQUESTS_PER_SECOND=5.0
      - CRW_CRAWLER__USER_AGENT=MyBot/1.0
      - CRW_AUTH__API_KEYS=["sk-production-key"]
      - CRW_RENDERER__LIGHTPANDA__WS_URL=ws://lightpanda:9222
```

## Production Tips

### Resource Limits

```yaml
services:
  crw:
    deploy:
      resources:
        limits:
          memory: 256M
          cpus: "1.0"
```

crw uses ~3 MB idle and ~66 MB under heavy load (50 concurrent requests), so 256 MB is generous.

### Health Check

```yaml
services:
  crw:
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:3000/health"]
      interval: 30s
      timeout: 5s
      retries: 3
```

### Persistent Config

```yaml
services:
  crw:
    volumes:
      - ./my-config.toml:/app/config.default.toml:ro
```

### Logging

```yaml
services:
  crw:
    environment:
      - RUST_LOG=crw_server=info,crw_renderer=warn
    logging:
      driver: json-file
      options:
        max-size: "10m"
        max-file: "3"
```
