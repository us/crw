---
title: Docker 部署
layout: default
nav_order: 6
description: "使用 Docker 部署 CRW。Docker Compose 设置，含 LightPanda JS 渲染、自定义配置和生产环境建议。"
parent: 首页
---

# Docker 部署
{: .no_toc }

使用 Docker Compose 运行 CRW，最简单的设置方式，内含 JS 渲染。
{: .fs-6 .fw-300 }

## 目录
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## 快速开始

```bash
git clone https://github.com/us/crw.git
cd crw
docker compose up
```

这将启动两个服务：

| 服务 | 端口 | 描述 |
|:-----|:-----|:-----|
| **crw** | 3000 | 启用 CDP 的 API 服务器 |
| **lightpanda** | 9222 | JS 渲染无头浏览器 |

验证：

```bash
curl http://localhost:3000/health
```

## Docker Compose

`docker-compose.yml`：

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

### 添加 Playwright（可选）

在 `docker-compose.yml` 中取消注释或添加：

```yaml
  playwright:
    image: mcr.microsoft.com/playwright:v1.49.0-noble
    command: ["npx", "playwright", "run-server", "--port=9223"]
    ports:
      - "9223:9223"
```

然后在 `crw` 服务中添加环境变量：

```yaml
    environment:
      - CRW_RENDERER__PLAYWRIGHT__WS_URL=ws://playwright:9223
```

## Dockerfile

多阶段构建，最小化镜像体积：

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

单独构建：

```bash
docker build -t crw .
docker run -p 3000:3000 crw
```

## 自定义配置

通过环境变量覆盖任何设置：

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

## 生产环境建议

### 资源限制

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

CRW 空闲时约 3 MB，高负载（50 并发请求）时约 66 MB，所以 256 MB 已经很充裕。

### 健康检查

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

### 持久化配置

挂载自定义配置文件：

```yaml
services:
  crw:
    build: .
    volumes:
      - ./my-config.toml:/app/config.default.toml:ro
```

### 日志

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
