---
title: 首页
layout: home
nav_order: 1
description: "CRW — 轻量级、兼容 Firecrawl 的 AI 网页抓取和爬虫工具。单一二进制文件，约 3MB 内存，LLM 结构化提取，Claude MCP 服务器，JS 渲染。"
permalink: /zh-CN/
---

# CRW
{: .fs-9 }

轻量级、兼容 Firecrawl 的 AI 网页抓取和爬虫工具。单一二进制文件，约 3 MB 空闲内存，LLM 结构化提取，Claude MCP 服务器。
{: .fs-6 .fw-300 }

[快速开始]({% link zh-CN/getting-started.md %}){: .btn .btn-primary .fs-5 .mb-4 .mb-md-0 .mr-2 }
[API 参考]({% link zh-CN/api-reference.md %}){: .btn .fs-5 .mb-4 .mb-md-0 }

[English]({% link index.md %}) | **中文**

---

## 为什么选择 CRW？

CRW 是 [Firecrawl](https://firecrawl.dev) 的**直接替代品**，可自行托管。使用 Rust 构建，性能卓越，资源占用极低 — 无需 Node.js，无需 Redis，只需一个二进制文件。

| | CRW | Firecrawl |
|---|---|---|
| **覆盖率（1K URL）** | **91.5%** | 77.2% |
| **平均延迟** | **833ms** | 4,600ms |
| **P50 延迟** | **446ms** | — |
| **噪声过滤率** | **89.1%** | — |
| **空闲内存** | 6.6 MB | ~500 MB+ |
| **冷启动** | 85 ms | 数秒 |
| **HTTP 抓取** | ~30 ms | ~200 ms+ |
| **二进制大小** | ~8 MB | Node.js 运行时 |
| **每千次成本** | **$0** | $0.83–5.33 |
| **依赖** | 单一二进制 | Node + Redis |
| **许可证** | MIT | AGPL |

基准测试：[Firecrawl scrape-content-dataset-v1](https://huggingface.co/datasets/firecrawl/scrape-content-dataset-v1) — 1,000 个真实 URL。

## 功能特性

- **🔌 兼容 Firecrawl API** — 相同端点、相同请求/响应格式，可直接替换
- **📄 6 种输出格式** — Markdown、HTML、清洁 HTML、原始 HTML、纯文本、链接、结构化 JSON
- **🤖 LLM 结构化提取** — 发送 JSON Schema，获取经验证的结构化数据（Anthropic tool_use + OpenAI function calling）
- **🌐 JS 渲染** — 通过 Shell 启发式自动检测 SPA，通过 LightPanda、Playwright 或 Chrome（CDP）渲染
- **🕷️ BFS 爬虫** — 异步爬取，支持速率限制、robots.txt、站点地图、并发任务
- **🔧 MCP 服务器** — 内置 stdio + HTTP 传输，支持 Claude Code 和 Claude Desktop
- **🔒 身份验证** — 可选的 Bearer Token，常量时间比较
- **🐳 Docker 就绪** — 多阶段构建，含 LightPanda 边车

## 使用场景

- **RAG 流水线** — 爬取网站并提取结构化数据用于向量数据库
- **AI 代理** — 通过 MCP 为 Claude Code 或 Claude Desktop 提供网页抓取工具
- **内容监控** — 定期爬取并使用 LLM 提取来跟踪变化
- **数据提取** — 结合 CSS 选择器 + LLM 从任何页面提取任意 Schema
- **网页归档** — 全站 BFS 爬取转为 Markdown

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
