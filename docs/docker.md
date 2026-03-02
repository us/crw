---
title: Docker
layout: default
nav_order: 6
description: "Deploy CRW with Docker. Docker Compose setup with LightPanda JS rendering, custom configuration, and production tips."
---

# Docker Deployment
{: .no_toc }

Run CRW with Docker Compose for the easiest setup with JS rendering included.
{: .fs-6 .fw-300 }

## Table of Contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Quick Start

```bash
git clone https://github.com/us/crw.git
cd crw
docker compose up
```

This starts two services:

| Service | Port | Description |
|:--------|:-----|:------------|
| **crw** | 3000 | API server with CDP enabled |
| **lightpanda** | 9222 | Headless browser for JS rendering |

Verify:

```bash
curl http://localhost:3000/health
```

## Docker Compose

`docker-compose.yml`:

```yaml
services:
  crw:
    build: .
    ports:
      - "3000:3000"
    depends_on:
      - lightpanda
    environment:
      - RUST_LOG=info
      - CRW_RENDERER__LIGHTPANDA__WS_URL=ws://lightpanda:9222

  lightpanda:
    image: lightpanda/browser:latest
    ports:
      - "9222:9222"
```

### Add Playwright (optional)

Uncomment in `docker-compose.yml` or add:

```yaml
  playwright:
    image: mcr.microsoft.com/playwright:v1.49.0-noble
    command: ["npx", "playwright", "run-server", "--port=9223"]
    ports:
      - "9223:9223"
```

Then add the env var to the `crw` service:

```yaml
    environment:
      - CRW_RENDERER__PLAYWRIGHT__WS_URL=ws://playwright:9223
```

## Dockerfile

Multi-stage build for minimal image size:

```dockerfile
FROM rust:1.83-bookworm AS builder
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

Build standalone:

```bash
docker build -t crw .
docker run -p 3000:3000 crw
```

## Custom Configuration

Override any setting via environment variables:

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
    build: .
    deploy:
      resources:
        limits:
          memory: 256M
          cpus: "1.0"
```

CRW uses ~3 MB idle and ~66 MB under heavy load (50 concurrent requests), so 256 MB is generous.

### Health Check

```yaml
services:
  crw:
    build: .
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:3000/health"]
      interval: 30s
      timeout: 5s
      retries: 3
```

### Persistent Config

Mount a custom config file:

```yaml
services:
  crw:
    build: .
    volumes:
      - ./my-config.toml:/app/config.default.toml:ro
```

### Logging

```yaml
services:
  crw:
    build: .
    environment:
      - RUST_LOG=crw_server=info,crw_renderer=warn
    logging:
      driver: json-file
      options:
        max-size: "10m"
        max-file: "3"
```
