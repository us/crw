# CRW

[English](README.md) | **中文**

轻量级、兼容 Firecrawl 的网页抓取工具。单一二进制文件，约 3MB 空闲内存，可选通过 LightPanda 进行 JS 渲染。

**API 兼容 [Firecrawl](https://firecrawl.dev)** — 可直接替换自托管部署。

## 快速开始

```bash
# 构建
cargo build --release --bin crw-server

# 运行
./target/release/crw-server

# 抓取网页
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

## 功能特性

- **4 个 API 端点** — `/v1/scrape`、`/v1/crawl`、`/v1/crawl/:id`、`/v1/map`
- **多种输出格式** — Markdown、HTML、纯文本、链接
- **JS 渲染** — 自动检测 SPA，通过 LightPanda/Playwright/Chrome（CDP）渲染
- **BFS 爬虫** — 异步爬取，支持速率限制、robots.txt、站点地图
- **LLM 结构化提取** — 通过 Claude 或 OpenAI 输出结构化 JSON
- **MCP 服务器** — 可作为 Claude Code / Claude Desktop 的工具使用
- **身份验证** — 可选的 Bearer Token 认证
- **Docker 就绪** — 多阶段 Dockerfile + docker-compose（含 LightPanda）

## API 端点

| 方法 | 端点 | 描述 |
|------|------|------|
| `POST` | `/v1/scrape` | 抓取单个 URL |
| `POST` | `/v1/crawl` | 启动异步爬取（返回任务 ID） |
| `GET` | `/v1/crawl/:id` | 查询爬取状态 / 获取结果 |
| `POST` | `/v1/map` | 发现网站上的所有 URL |
| `GET` | `/health` | 健康检查（无需认证） |

## MCP 服务器（Claude Code / Desktop）

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

工具：`crw_scrape`、`crw_crawl`、`crw_check_crawl_status`、`crw_map`

## Docker

```bash
docker compose up
```

## 性能指标

| 指标 | 数值 |
|------|------|
| 空闲内存 | 3.3 MB（服务器）+ 3.3 MB（LightPanda） |
| HTTP 抓取 | 平均约 30ms |
| JS 抓取 | 平均约 520ms |
| 冷启动 | 约 85ms |

## 文档

完整文档：**[docs/index.md](docs/index.md)** | **[中文文档](docs/zh-CN/index.md)**

- [安装指南](docs/zh-CN/getting-started.md)
- [配置说明](docs/zh-CN/configuration.md)
- [API 参考](docs/zh-CN/api-reference.md)
- [MCP 服务器](docs/zh-CN/mcp-server.md)
- [Docker 部署](docs/zh-CN/docker.md)

## 许可证

MIT
