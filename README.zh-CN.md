# CRW

**轻量级、兼容 Firecrawl 的 AI 网页抓取和爬虫工具**

CRW 是一个基于 Rust 构建的自托管网页抓取和爬虫工具 — 专为 LLM 结构化提取、RAG 流水线和 AI 代理设计的快速、轻量级 Firecrawl 替代方案。单一二进制文件，约 3 MB 空闲内存，内置 MCP 服务器支持 Claude，通过 Anthropic 和 OpenAI 进行结构化数据提取。完全兼容 Firecrawl API。

[English](README.md) | **中文**

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE-MIT)
[![Rust](https://img.shields.io/badge/Rust-2024_edition-orange.svg)](https://www.rust-lang.org/)

## 为什么选择 CRW？

CRW 提供 Firecrawl 的 API，但资源占用极低。无运行时依赖，无 Redis，无 Node.js — 只需一个二进制文件即可部署到任何地方。

| | CRW | Firecrawl |
|---|---|---|
| **覆盖率（1K URL）** | **92.0%** | 77.2% |
| **平均延迟** | **833ms** | 4,600ms |
| **P50 延迟** | **446ms** | — |
| **噪声过滤率** | **88.4%** | — |
| **空闲内存** | 6.6 MB | ~500 MB+ |
| **冷启动** | 85 ms | 数秒 |
| **HTTP 抓取** | ~30 ms | ~200 ms+ |
| **二进制大小** | ~8 MB | Node.js 运行时 |
| **每千次成本** | **$0**（自托管） | $0.83–5.33 |
| **依赖** | 单一二进制 | Node + Redis |
| **许可证** | MIT | AGPL |

基准测试：[Firecrawl scrape-content-dataset-v1](https://huggingface.co/datasets/firecrawl/scrape-content-dataset-v1) — 1,000 个真实 URL，启用 JS 渲染。

## 功能特性

- **🔌 兼容 Firecrawl API** — 相同端点、相同请求/响应格式，可直接替换
- **📄 6 种输出格式** — Markdown、HTML、清洁 HTML、原始 HTML、纯文本、链接、结构化 JSON
- **🤖 LLM 结构化提取** — 发送 JSON Schema，获取经验证的结构化数据（Anthropic tool_use + OpenAI function calling）
- **🌐 JS 渲染** — 通过 Shell 启发式自动检测 SPA，通过 LightPanda、Playwright 或 Chrome（CDP）渲染
- **🕷️ BFS 爬虫** — 异步爬取，支持速率限制、robots.txt、站点地图、并发任务
- **🔧 MCP 服务器** — 内置 stdio + HTTP 传输，支持 Claude Code 和 Claude Desktop
- **🔒 身份验证** — 可选的 Bearer Token，常量时间比较
- **🐳 Docker 就绪** — 多阶段构建，含 LightPanda 边车

## 快速开始

**从源码构建：**

```bash
cargo build --release --bin crw-server
./target/release/crw-server
```

**Docker：**

```bash
docker compose up
```

**抓取网页：**

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

## 使用场景

- **RAG 流水线** — 爬取网站并提取结构化数据用于向量数据库
- **AI 代理** — 通过 MCP 为 Claude Code 或 Claude Desktop 提供网页抓取工具
- **内容监控** — 定期爬取并使用 LLM 提取来跟踪变化
- **数据提取** — 结合 CSS 选择器 + LLM 从任何页面提取任意 Schema
- **网页归档** — 全站 BFS 爬取转为 Markdown

## API 端点

| 方法 | 端点 | 描述 |
|------|------|------|
| `POST` | `/v1/scrape` | 抓取单个 URL，可选 LLM 提取 |
| `POST` | `/v1/crawl` | 启动异步 BFS 爬取（返回任务 ID） |
| `GET` | `/v1/crawl/:id` | 查询爬取状态并获取结果 |
| `POST` | `/v1/map` | 发现网站上的所有 URL |
| `GET` | `/health` | 健康检查（无需认证） |
| `POST` | `/mcp` | Streamable HTTP MCP 传输 |

## LLM 结构化提取

在抓取请求中发送 JSON Schema，CRW 将使用 LLM 函数调用返回经验证的结构化数据。

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

- **Anthropic** — 使用 `tool_use` 的 `input_schema` 进行提取
- **OpenAI** — 使用 function calling 的 `parameters` Schema
- **验证** — LLM 输出在返回前会根据你的 JSON Schema 进行验证

在配置中设置 LLM 提供商：

```toml
[extraction.llm]
provider = "anthropic"        # "anthropic" 或 "openai"
api_key = "sk-..."            # 或 CRW_EXTRACTION__LLM__API_KEY 环境变量
model = "claude-sonnet-4-20250514"
```

## MCP 服务器

CRW 可作为 Claude Code 和 Claude Desktop 的 MCP 工具服务器，支持两种传输方式。

**HTTP 传输（推荐）：**

```bash
claude mcp add --transport http crw http://localhost:3000/mcp
```

**Stdio 传输：**

```bash
cargo build --release --bin crw-mcp
```

添加到 `~/.claude.json`：

```json
{
  "mcpServers": {
    "crw": {
      "command": "/absolute/path/to/crw-mcp",
      "env": {
        "CRW_API_URL": "http://localhost:3000"
      }
    }
  }
}
```

**工具：** `crw_scrape`、`crw_crawl`、`crw_check_crawl_status`、`crw_map`

## JS 渲染

CRW 通过分析初始 HTML 响应的 Shell 启发式方法（空 body、框架标记）自动检测 SPA。检测到 SPA 时，会通过无头浏览器渲染页面。

**支持的渲染器：**

| 渲染器 | 协议 | 最适用于 |
|--------|------|----------|
| LightPanda | CDP over WebSocket | 低资源环境（默认） |
| Playwright | CDP over WebSocket | 完整浏览器兼容性 |
| Chrome | CDP over WebSocket | 现有 Chrome 基础设施 |

渲染器模式通过 `renderer.mode` 配置：`auto`（默认）、`lightpanda`、`playwright`、`chrome` 或 `none`。

使用 Docker Compose 时，LightPanda 作为边车运行 — 无需额外设置：

```bash
docker compose up
```

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

查看[完整配置参考](docs/zh-CN/configuration.md)了解所有选项。

## 集成示例

**Python：**

```python
import requests

response = requests.post("http://localhost:3000/v1/scrape", json={
    "url": "https://example.com",
    "formats": ["markdown", "links"]
})
data = response.json()["data"]
print(data["markdown"])
```

**Node.js：**

```javascript
const response = await fetch("http://localhost:3000/v1/scrape", {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({
    url: "https://example.com",
    formats: ["markdown", "links"]
  })
});
const { data } = await response.json();
console.log(data.markdown);
```

**LangChain 文档加载器模式：**

```python
import requests

def load_documents(urls):
    documents = []
    for url in urls:
        resp = requests.post("http://localhost:3000/v1/scrape", json={
            "url": url,
            "formats": ["markdown"]
        })
        data = resp.json()["data"]
        documents.append({
            "page_content": data["markdown"],
            "metadata": data["metadata"]
        })
    return documents
```

## Docker

```bash
docker compose up
```

这将在端口 `3000` 启动 CRW，并在端口 `9222` 启动 LightPanda 作为 JS 渲染边车。CRW 会自动连接到 LightPanda 进行 SPA 渲染。

## 文档

完整文档：**[docs/index.md](docs/index.md)** | **[中文文档](docs/zh-CN/index.md)**

- [安装指南](docs/zh-CN/getting-started.md)
- [配置说明](docs/zh-CN/configuration.md)
- [API 参考](docs/zh-CN/api-reference.md)
- [MCP 服务器](docs/zh-CN/mcp-server.md)
- [Docker 部署](docs/zh-CN/docker.md)

## 贡献

欢迎贡献！请提交 issue 或 pull request。

1. Fork 仓库
2. 创建功能分支（`git checkout -b feat/my-feature`）
3. 提交更改（`git commit -m 'feat: add my feature'`）
4. 推送到分支（`git push origin feat/my-feature`）
5. 创建 Pull Request

## 许可证

[MIT](LICENSE-MIT)
