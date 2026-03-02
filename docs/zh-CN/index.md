---
title: 首页
layout: home
nav_order: 1
description: "CRW — 轻量级、兼容 Firecrawl 的网页抓取工具。单一二进制文件，约 3MB 内存，JS 渲染，Claude MCP 服务器。"
permalink: /zh-CN/
---

# CRW
{: .fs-9 }

轻量级、兼容 Firecrawl 的网页抓取工具。单一二进制文件，约 3MB 空闲内存，可选通过 LightPanda 进行 JS 渲染。
{: .fs-6 .fw-300 }

[快速开始]({% link zh-CN/getting-started.md %}){: .btn .btn-primary .fs-5 .mb-4 .mb-md-0 .mr-2 }
[API 参考]({% link zh-CN/api-reference.md %}){: .btn .fs-5 .mb-4 .mb-md-0 }

[English]({% link index.md %}) | **中文**

---

## 为什么选择 CRW？

CRW 是 [Firecrawl](https://firecrawl.dev) 的**直接替代品**，可自行托管。使用 Rust 构建，性能卓越，资源占用极低。

| | CRW | Firecrawl |
|---|---|---|
| **空闲内存** | 3.3 MB | ~500 MB+ |
| **冷启动** | 85ms | 数秒 |
| **HTTP 抓取** | ~30ms | ~200ms+ |
| **二进制大小** | ~8 MB | Node.js 运行时 |
| **依赖** | 单一二进制 | Node、Redis 等 |
| **许可证** | MIT | AGPL |

## 功能特性

- **兼容 Firecrawl API** — 相同端点、相同请求/响应格式
- **4 个端点** — `/v1/scrape`、`/v1/crawl`、`/v1/crawl/:id`、`/v1/map`
- **多种输出格式** — Markdown、HTML、清理后的 HTML、纯文本、链接
- **JS 渲染** — 自动检测 SPA，通过 LightPanda、Playwright 或 Chrome 渲染
- **BFS 爬虫** — 异步爬取，支持速率限制、robots.txt、站点地图
- **LLM 结构化提取** — 通过 Claude 或 OpenAI 输出结构化 JSON
- **MCP 服务器** — 可在 Claude Code 或 Claude Desktop 中作为工具使用
- **身份验证** — 可选的 Bearer Token 认证
- **Docker 就绪** — 多阶段 Dockerfile + docker-compose（含 LightPanda）

## 快速示例

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

## 架构

```
crates/
  crw-core      类型定义、配置、错误类型
  crw-extract   HTML 清理、可读性提取、Markdown 转换、LLM 提取
  crw-renderer  HTTP 抓取器、CDP 客户端（tokio-tungstenite）
  crw-crawl     单页抓取、BFS 爬虫、速率限制、robots.txt
  crw-server    Axum HTTP 服务器、路由、认证中间件
  crw-mcp       MCP stdio 服务器（JSON-RPC 2.0 代理）
```
