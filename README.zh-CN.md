<p align="center">
  <h1 align="center">CRW</h1>
  <p align="center">轻量级、兼容 Firecrawl 的 AI 网页抓取和爬虫工具</p>
  <p align="center">
    <a href="https://crates.io/crates/crw-server"><img src="https://img.shields.io/crates/v/crw-server.svg" alt="crates.io"></a>
    <a href="https://github.com/us/crw/actions"><img src="https://github.com/us/crw/workflows/CI/badge.svg" alt="CI"></a>
    <a href="LICENSE"><img src="https://img.shields.io/badge/license-AGPL--3.0-blue.svg" alt="License"></a>
    <a href="https://github.com/us/crw/stargazers"><img src="https://img.shields.io/github/stars/us/crw?style=social" alt="GitHub Stars"></a>
    <a href="https://fastcrw.com"><img src="https://img.shields.io/badge/Managed%20Cloud-fastcrw.com-blueviolet" alt="fastcrw.com"></a>
  </p>
  <p align="center">
    <a href="https://fastcrw.com">云服务</a> &bull;
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

> **不想自托管？** [fastcrw.com](https://fastcrw.com) 是托管云服务 — 全球代理网络、自动扩展、仪表板和 API 密钥。相同的 Firecrawl 兼容 API。[获取 50 个免费额度 →](https://fastcrw.com)

CRW 是一个基于 Rust 构建的自托管网页抓取和爬虫工具 — 可直接替换 Firecrawl，专为 LLM 结构化提取、RAG 流水线和 AI 代理设计。单一二进制文件，约 6 MB 空闲内存，内置 MCP 服务器支持 Claude，通过 Anthropic 和 OpenAI 进行结构化数据提取。

**单一二进制文件。无 Redis。无 Node.js。兼容 Firecrawl API。**

```bash
cargo install crw-server
crw-server
```

## 最新动态

### v0.0.2

- **CSS 选择器 & XPath** — Markdown 转换前提取特定 DOM 元素（`cssSelector`、`xpath`）
- **分块策略** — 按主题、句子或正则表达式将内容切分为 RAG 流水线所需的块（`chunkStrategy`）
- **BM25 & 余弦过滤** — 按查询相关性对块排序，返回前 K 个结果（`filterMode`、`topK`）
- **更好的 Markdown** — 切换到 `htmd`（Turndown.js 移植版）：表格、代码块语言、嵌套列表均正确渲染
- **隐身模式** — 从内置 Chrome/Firefox/Safari 池轮换 User-Agent，注入 12 个浏览器同款请求头（`stealth: true`）
- **单请求代理** — 每次请求可单独覆盖全局代理（`proxy: "http://..."`）
- **速率限制抖动** — 请求间随机延迟，避免均匀流量指纹
- **`crw-server setup`** — 一键 JS 渲染设置：自动下载 LightPanda，创建 `config.local.toml`

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

| | CRW（自托管） | fastcrw.com（云服务） | Firecrawl |
|---|---|---|---|
| **覆盖率（1K URL）** | **92.0%** | **92.0%** | 77.2% |
| **平均延迟** | **833ms** | **833ms** | 4,600ms |
| **P50 延迟** | **446ms** | **446ms** | — |
| **噪声过滤率** | **88.4%** | **88.4%** | — |
| **空闲内存** | 6.6 MB | 0（托管） | ~500 MB+ |
| **冷启动** | 85 ms | 0（始终在线） | 数秒 |
| **代理网络** | 自备 | 全球（内置） | 内置 |
| **仪表板** | — | 有 | 有 |
| **每千次成本** | **$0**（自托管） | 从 $13/月起 | $0.83–5.33 |
| **依赖** | 单一二进制 | 无（API） | Node + Redis |
| **许可证** | AGPL-3.0 | 托管 | AGPL |

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

## 云服务 vs 自托管

| | 自托管 | 云服务（[fastcrw.com](https://fastcrw.com)） |
|---|---|---|
| **部署** | `cargo install crw-server` | 注册 → 获取 API 密钥 |
| **基础设施** | 自行管理 | 完全托管 |
| **代理** | 自备 | 全球代理网络 |
| **扩展** | 手动 | 自动扩展 |
| **API** | 兼容 Firecrawl | 相同的 Firecrawl 兼容 API |

两者使用相同的 Firecrawl 兼容 API — 只需更改 base URL 即可在自托管和云服务之间切换。

## 快速开始

**云服务（无需部署）：**

```bash
curl -X POST https://fastcrw.com/api/v1/scrape \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'
```

> 在 [fastcrw.com](https://fastcrw.com) 获取 API 密钥 — 包含 50 个免费额度。

**自托管 — 从 crates.io 安装：**

```bash
cargo install crw-server
crw-server
```

**启用 JS 渲染（可选）：**

```bash
crw-server setup
```

自动下载 [LightPanda](https://github.com/lightpanda-io/browser) 并创建 `config.local.toml` 配置文件。详见 [JS 渲染](#js-渲染)。

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

**快速设置（推荐）：**

```bash
crw-server setup
```

自动下载 LightPanda 二进制文件到 `~/.local/bin/` 并创建正确的渲染器配置。然后启动 LightPanda 和 CRW：

```bash
lightpanda serve --host 127.0.0.1 --port 9222 &
crw-server
```

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

## Crates

| Crate | 描述 | |
|-------|------|-|
| [`crw-core`](crates/crw-core) | 核心类型、配置和错误处理 | [![crates.io](https://img.shields.io/crates/v/crw-core.svg)](https://crates.io/crates/crw-core) |
| [`crw-renderer`](crates/crw-renderer) | HTTP + CDP 浏览器渲染引擎 | [![crates.io](https://img.shields.io/crates/v/crw-renderer.svg)](https://crates.io/crates/crw-renderer) |
| [`crw-extract`](crates/crw-extract) | HTML → Markdown/纯文本提取 | [![crates.io](https://img.shields.io/crates/v/crw-extract.svg)](https://crates.io/crates/crw-extract) |
| [`crw-crawl`](crates/crw-crawl) | 异步 BFS 爬虫，支持 robots.txt 和站点地图 | [![crates.io](https://img.shields.io/crates/v/crw-crawl.svg)](https://crates.io/crates/crw-crawl) |
| [`crw-server`](crates/crw-server) | Axum API 服务器（兼容 Firecrawl） | [![crates.io](https://img.shields.io/crates/v/crw-server.svg)](https://crates.io/crates/crw-server) |
| [`crw-mcp`](crates/crw-mcp) | MCP stdio 代理二进制文件 | [![crates.io](https://img.shields.io/crates/v/crw-mcp.svg)](https://crates.io/crates/crw-mcp) |

详细用法和 `cargo add` 命令请参见 [docs/crates.md](docs/crates.md)。

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
2. 安装 pre-commit hooks：`make hooks`
3. 创建功能分支（`git checkout -b feat/my-feature`）
4. 提交更改（`git commit -m 'feat: add my feature'`）
5. 推送到分支（`git push origin feat/my-feature`）
6. 创建 Pull Request

Pre-commit hook 会运行与 CI 相同的检查（`cargo fmt`、`cargo clippy`、`cargo test`）。也可以通过 `make check` 手动运行。

## 许可证

CRW 基于 [AGPL-3.0](LICENSE) 开源。如需无 AGPL 义务的托管版本，请访问 [fastcrw.com](https://fastcrw.com)。
