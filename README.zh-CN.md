<p align="center">
  <h1 align="center">CRW</h1>
  <p align="center">轻量级、兼容 Firecrawl 的 AI 网页抓取和爬虫工具</p>
  <p align="center">
    <a href="docs/docs/installation.md">安装指南</a> &bull;
    <a href="docs/docs/rest-api.md">API 参考</a> &bull;
    <a href="docs/docs/mcp.md">MCP 集成</a> &bull;
    <a href="docs/docs/js-rendering.md">JS 渲染</a> &bull;
    <a href="docs/docs/configuration.md">配置说明</a>
  </p>
  <p align="center">
    <a href="README.md">English</a> | <b>中文</b>
  </p>
</p>

---

CRW 是一个基于 Rust 构建的自托管网页抓取和爬虫工具 — 可直接替换 Firecrawl，专为 LLM 结构化提取、RAG 流水线和 AI 代理设计。单一二进制文件，约 6 MB 空闲内存，内置 MCP 服务器支持 Claude，通过 Anthropic 和 OpenAI 进行结构化数据提取。

**单一二进制文件。无 Redis。无 Node.js。兼容 Firecrawl API。**

```bash
cargo install crw-server
crw-server
```

## 最新动态

### `crw-server setup` 命令

- **一键 JS 渲染设置** — `crw-server setup` 自动下载 LightPanda 并创建 `config.local.toml`
- **平台检测** — 检测操作系统和架构，下载正确的二进制文件（Linux x86_64、macOS aarch64）
- **CLI 子命令** — crw-server 现在使用 clap 支持可扩展的子命令

### v0.0.1

- **兼容 Firecrawl 的 REST API** — `/v1/scrape`、`/v1/crawl`、`/v1/map`，请求/响应格式完全一致
- **6 种输出格式** — Markdown、HTML、清洁 HTML、原始 HTML、纯文本、链接、结构化 JSON
- **LLM 结构化提取** — 发送 JSON Schema，获取经验证的结构化数据（Anthropic tool_use + OpenAI function calling）
- **JS 渲染** — 通过启发式方法自动检测 SPA，通过 LightPanda、Playwright 或 Chrome（CDP）渲染
- **BFS 爬虫** — 异步爬取，支持速率限制、robots.txt、站点地图、并发任务
- **MCP 服务器** — 内置 stdio + HTTP 传输，支持 Claude Code 和 Claude Desktop
- **SSRF 防护** — 私有 IP、云元数据、IPv6、危险 URI 过滤
- **Docker 就绪** — 多阶段构建，含 LightPanda 边车

## 为什么选择 CRW？

CRW 提供 Firecrawl 的 API，但资源占用极低。无运行时依赖，无 Redis，无 Node.js — 只需一个二进制文件即可部署到任何地方。

| | **CRW** | Firecrawl |
|---|---|---|
| **覆盖率（1K URL）** | **92.0%** | 77.2% |
| **平均延迟** | **833ms** | 4,600ms |
| **P50 延迟** | **446ms** | — |
| **噪声过滤率** | **88.4%** | — |
| **空闲内存** | **6.6 MB** | ~500 MB+ |
| **冷启动** | **85 ms** | 数秒 |
| **HTTP 抓取** | **~30 ms** | ~200 ms+ |
| **二进制大小** | **~8 MB** | Node.js 运行时 |
| **每千次成本** | **$0**（自托管） | $0.83–5.33 |
| **依赖** | 单一二进制 | Node + Redis |

基准测试：[Firecrawl scrape-content-dataset-v1](https://huggingface.co/datasets/firecrawl/scrape-content-dataset-v1) — 1,000 个真实 URL，启用 JS 渲染。

## 快速开始

### 安装和运行

```bash
cargo install crw-server
crw-server
# 服务器启动在 http://localhost:3000
```

### 启用 JS 渲染（可选）

```bash
crw-server setup
lightpanda serve --host 127.0.0.1 --port 9222 &
crw-server
```

### Docker

```bash
# 预构建镜像
docker run -p 3000:3000 ghcr.io/us/crw:latest

# 含 JS 渲染边车
docker compose up
```

### 抓取网页

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'
```

```json
{
  "success": true,
  "data": {
    "markdown": "# Example Domain\nThis domain is for use in ...",
    "metadata": {
      "title": "Example Domain",
      "sourceURL": "https://example.com",
      "statusCode": 200,
      "elapsedMs": 32
    }
  }
}
```

### 爬取网站

```bash
# 启动异步爬取
curl -X POST http://localhost:3000/v1/crawl \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com", "maxDepth": 2, "maxPages": 50}'

# 查询状态
curl http://localhost:3000/v1/crawl/<job-id>
```

### LLM 结构化提取

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com/product",
    "formats": ["json"],
    "jsonSchema": {
      "type": "object",
      "properties": {
        "name": { "type": "string" },
        "price": { "type": "number" }
      },
      "required": ["name", "price"]
    }
  }'
```

配置提供商：

```toml
[extraction.llm]
provider = "anthropic"        # "anthropic" 或 "openai"
api_key = "sk-..."            # 或 CRW_EXTRACTION__LLM__API_KEY 环境变量
model = "claude-sonnet-4-20250514"
```

### 配合 MCP 使用（Claude Code、Cursor）

```bash
# HTTP 传输（推荐）
claude mcp add --transport http crw http://localhost:3000/mcp

# Stdio 传输
cargo install crw-mcp
```

添加到 AI 工具的 MCP 配置：

```json
{
  "mcpServers": {
    "crw": {
      "command": "crw-mcp",
      "env": { "CRW_API_URL": "http://localhost:3000" }
    }
  }
}
```

工具：`crw_scrape`、`crw_crawl`、`crw_check_crawl_status`、`crw_map`

详见 [MCP 设置指南](docs/docs/mcp.md)。

## 功能特性

| 功能 | 描述 |
|------|------|
| **Firecrawl API** | 兼容 `/v1/scrape`、`/v1/crawl`、`/v1/map` 端点 |
| **6 种输出格式** | Markdown、HTML、清洁 HTML、原始 HTML、纯文本、链接、结构化 JSON |
| **LLM 提取** | 发送 JSON Schema，获取经验证的结构化数据（Anthropic + OpenAI） |
| **JS 渲染** | 自动检测 SPA，通过 LightPanda、Playwright 或 Chrome（CDP）渲染 |
| **BFS 爬虫** | 异步爬取，支持速率限制、robots.txt、站点地图、并发任务 |
| **MCP 服务器** | 内置 stdio + HTTP 传输，支持 Claude Code 和 Claude Desktop |
| **一键设置** | `crw-server setup` 下载 LightPanda 并创建配置 |
| **SSRF 防护** | 私有 IP、云元数据、IPv6、危险 URI 过滤 |
| **身份验证** | 可选 Bearer Token，常量时间比较 |
| **Docker** | 多阶段构建，含 LightPanda 边车 |

## 安全性

CRW 内置了针对常见网页抓取攻击向量的保护措施：

- **SSRF 防护** — 所有 URL 输入（REST API + MCP）都会验证是否为私有/内部网络：
  - 回环地址（`127.0.0.0/8`、`::1`、`localhost`）
  - 私有 IP（`10.0.0.0/8`、`172.16.0.0/12`、`192.168.0.0/16`）
  - 链路本地 / 云元数据（`169.254.0.0/16` — 阻止 AWS/GCP 元数据端点）
  - IPv6 映射地址（`::ffff:127.0.0.1`）、链路本地（`fe80::`）、ULA（`fc00::/7`）
  - 非 HTTP 协议（`file://`、`ftp://`、`gopher://`、`data:`）
- **身份验证** — 可选 Bearer Token，常量时间比较（无长度或密钥索引泄露）
- **robots.txt** — 遵守 `Allow`/`Disallow`，支持通配符（`*`、`$`）和 RFC 9309 特异性规则
- **速率限制** — 可配置的每秒请求上限
- **资源限制** — 最大正文 1 MB、最大爬取深度 10、最大页面数 1000、最大发现 URL 5000

## 架构

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

## 配置

CRW 使用分层 TOML 配置，支持环境变量覆盖：

1. `config.default.toml` — 内置默认值
2. `config.local.toml` — 本地覆盖（或设置 `CRW_CONFIG=myconfig`）
3. 环境变量 — `CRW_` 前缀，`__` 分隔符（例如 `CRW_SERVER__PORT=8080`）

```toml
[server]
host = "0.0.0.0"
port = 3000

[renderer]
mode = "auto"  # auto | lightpanda | playwright | chrome | none

[crawler]
max_concurrency = 10
requests_per_second = 10.0
respect_robots_txt = true

[auth]
# api_keys = ["fc-key-1234"]
```

详见[配置指南](docs/docs/configuration.md)。

## Crates

| Crate | 描述 | |
|-------|------|-|
| [`crw-core`](crates/crw-core) | 核心类型、配置和错误处理 | [![crates.io](https://img.shields.io/crates/v/crw-core.svg)](https://crates.io/crates/crw-core) |
| [`crw-renderer`](crates/crw-renderer) | HTTP + CDP 浏览器渲染引擎 | [![crates.io](https://img.shields.io/crates/v/crw-renderer.svg)](https://crates.io/crates/crw-renderer) |
| [`crw-extract`](crates/crw-extract) | HTML → Markdown/纯文本提取 | [![crates.io](https://img.shields.io/crates/v/crw-extract.svg)](https://crates.io/crates/crw-extract) |
| [`crw-crawl`](crates/crw-crawl) | 异步 BFS 爬虫，支持 robots.txt 和站点地图 | [![crates.io](https://img.shields.io/crates/v/crw-crawl.svg)](https://crates.io/crates/crw-crawl) |
| [`crw-server`](crates/crw-server) | Axum API 服务器（兼容 Firecrawl） | [![crates.io](https://img.shields.io/crates/v/crw-server.svg)](https://crates.io/crates/crw-server) |
| [`crw-mcp`](crates/crw-mcp) | MCP stdio 代理二进制文件 | [![crates.io](https://img.shields.io/crates/v/crw-mcp.svg)](https://crates.io/crates/crw-mcp) |

## 文档

- [安装指南](docs/docs/installation.md) — 从 crates.io、源码或 Docker 安装
- [快速开始](docs/docs/quick-start.md) — 30 秒内完成第一次抓取
- [REST API](docs/docs/rest-api.md) — 完整端点参考
- [抓取](docs/docs/scraping.md) — 输出格式、选择器、LLM 提取
- [爬取](docs/docs/crawling.md) — BFS 爬虫、深度/页面限制、站点地图
- [JS 渲染](docs/docs/js-rendering.md) — LightPanda、Playwright、Chrome 设置
- [MCP 集成](docs/docs/mcp.md) — Claude Code、Cursor、Windsurf 等
- [配置说明](docs/docs/configuration.md) — 所有配置选项
- [Docker](docs/docs/docker.md) — 容器部署
- [架构](docs/docs/architecture.md) — 内部设计和 crate 结构

## 贡献

欢迎贡献！请提交 issue 或 pull request。

```bash
git clone https://github.com/us/crw
cd crw
cargo build --release
cargo test --workspace
```

## 许可证

AGPL-3.0 — 详见 [LICENSE](LICENSE)。
