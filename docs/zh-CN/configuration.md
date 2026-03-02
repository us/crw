---
title: 配置说明
layout: default
nav_order: 3
description: "CRW 网页抓取工具配置：环境变量、配置文件、渲染器设置、身份验证、LLM 提取。"
parent: 首页
---

# 配置说明
{: .no_toc }

CRW 从 `config.default.toml` 加载配置，然后通过环境变量覆盖。
{: .fs-6 .fw-300 }

## 目录
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## 环境变量

所有环境变量使用 `CRW_` 前缀，`__` 作为嵌套分隔符。

```bash
# 示例：将端口更改为 8080
CRW_SERVER__PORT=8080 ./target/release/crw-server
```

### 服务器

| 变量 | 默认值 | 描述 |
|:-----|:-------|:-----|
| `CRW_SERVER__HOST` | `0.0.0.0` | 绑定地址 |
| `CRW_SERVER__PORT` | `3000` | 绑定端口 |
| `CRW_SERVER__REQUEST_TIMEOUT_SECS` | `60` | 全局请求超时（秒） |

### 渲染器

| 变量 | 默认值 | 描述 |
|:-----|:-------|:-----|
| `CRW_RENDERER__MODE` | `auto` | 渲染模式：`auto` 或 `none` |
| `CRW_RENDERER__PAGE_TIMEOUT_MS` | `30000` | JS 页面渲染超时（毫秒） |
| `CRW_RENDERER__LIGHTPANDA__WS_URL` | — | LightPanda CDP WebSocket URL |
| `CRW_RENDERER__PLAYWRIGHT__WS_URL` | — | Playwright CDP WebSocket URL |
| `CRW_RENDERER__CHROME__WS_URL` | — | Chrome/Chromium CDP WebSocket URL |

### 爬虫

| 变量 | 默认值 | 描述 |
|:-----|:-------|:-----|
| `CRW_CRAWLER__MAX_CONCURRENCY` | `10` | 最大并行爬取请求数 |
| `CRW_CRAWLER__REQUESTS_PER_SECOND` | `10.0` | 速率限制（令牌桶） |
| `CRW_CRAWLER__RESPECT_ROBOTS_TXT` | `true` | 遵守 robots.txt 指令 |
| `CRW_CRAWLER__USER_AGENT` | `CRW/0.1` | User-Agent 请求头 |
| `CRW_CRAWLER__DEFAULT_MAX_DEPTH` | `2` | 默认爬取深度限制 |
| `CRW_CRAWLER__DEFAULT_MAX_PAGES` | `100` | 每次爬取默认最大页面数 |
| `CRW_CRAWLER__PROXY` | — | HTTP/HTTPS 代理 URL |
| `CRW_CRAWLER__JOB_TTL_SECS` | `3600` | 已完成任务清理 TTL |

### 内容提取

| 变量 | 默认值 | 描述 |
|:-----|:-------|:-----|
| `CRW_EXTRACTION__DEFAULT_FORMAT` | `markdown` | 默认输出格式 |
| `CRW_EXTRACTION__ONLY_MAIN_CONTENT` | `true` | 去除导航栏、页脚、侧边栏 |

### LLM 结构化提取

| 变量 | 默认值 | 描述 |
|:-----|:-------|:-----|
| `CRW_EXTRACTION__LLM__PROVIDER` | `anthropic` | `anthropic` 或 `openai` |
| `CRW_EXTRACTION__LLM__API_KEY` | — | LLM API 密钥（JSON 提取必需） |
| `CRW_EXTRACTION__LLM__MODEL` | `claude-sonnet-4-20250514` | 模型名称 |
| `CRW_EXTRACTION__LLM__MAX_TOKENS` | `4096` | 最大响应 Token 数 |
| `CRW_EXTRACTION__LLM__BASE_URL` | — | 自定义 API 端点（兼容 OpenAI） |

### 身份验证

| 变量 | 默认值 | 描述 |
|:-----|:-------|:-----|
| `CRW_AUTH__API_KEYS` | `[]` | 有效 Bearer Token 的 JSON 数组 |

---

## 配置文件

默认配置文件 `config.default.toml`：

```toml
[server]
host = "0.0.0.0"
port = 3000
request_timeout_secs = 60

[renderer]
mode = "auto"                     # auto | none
page_timeout_ms = 30000

[renderer.lightpanda]
ws_url = "ws://127.0.0.1:9222"

# [renderer.playwright]
# ws_url = "ws://playwright:9222"

# [renderer.chrome]
# ws_url = "ws://chrome:9222"

[crawler]
max_concurrency = 10
requests_per_second = 10.0
respect_robots_txt = true
user_agent = "CRW/0.1"
default_max_depth = 2
default_max_pages = 100
job_ttl_secs = 3600
# proxy = "http://proxy:8080"

[extraction]
default_format = "markdown"
only_main_content = true

[auth]
# api_keys = ["your-api-key"]

# [extraction.llm]
# provider = "anthropic"
# api_key = "sk-..."
# model = "claude-sonnet-4-20250514"
# max_tokens = 4096
# base_url = "https://custom-endpoint.example.com"
```

{: .note }
环境变量始终优先于配置文件。这使得在不同环境中使用相同配置文件变得简单，只需通过环境变量在生产环境中自定义即可。

---

## 身份验证

设置 `api_keys` 后，所有 `/v1/*` 端点都需要有效的 Bearer Token。`/health` 端点始终公开。

```bash
# 启用一个或多个密钥的身份验证
CRW_AUTH__API_KEYS='["sk-key-1", "sk-key-2"]' ./target/release/crw-server
```

客户端必须在 `Authorization` 请求头中发送 Token：

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer sk-key-1" \
  -d '{"url": "https://example.com"}'
```

无有效 Token 时：

```json
{
  "success": false,
  "error": "Missing Authorization header"
}
```

{: .note }
Token 比较使用恒定时间相等性检查，以防止时序攻击。

---

## JS 渲染

CRW 通过 CDP（Chrome DevTools Protocol）支持 JavaScript 渲染 SPA 单页应用。

### 支持的渲染器

| 渲染器 | 描述 | 空闲内存 |
|:-------|:-----|:---------|
| [LightPanda](https://github.com/nicholasgasior/lightpanda) | 轻量级无头浏览器 | ~3 MB |
| Playwright | 完整浏览器自动化 | ~200 MB |
| Chrome/Chromium | 标准无头 Chrome | ~150 MB |

### 使用 LightPanda 设置

```bash
# 启动 LightPanda
lightpanda serve --host 127.0.0.1 --port 9222

# 启动 CRW（必须使用 cdp 特性构建）
CRW_RENDERER__LIGHTPANDA__WS_URL=ws://127.0.0.1:9222 ./target/release/crw-server
```

{: .warning }
服务器二进制文件必须使用 `cdp` 特性构建：`cargo build --release --bin crw-server --features crw-server/cdp`

### 渲染模式

**服务器级别**（配置）：

| 模式 | 行为 |
|:-----|:-----|
| `auto`（默认） | 先 HTTP，自动检测 SPA shell，需要时用 JS 重试 |
| `none` | 仅 HTTP，禁用所有 JS 渲染器 |

**每请求**（`renderJs` 字段）：

| 值 | 行为 |
|:---|:-----|
| `null`（默认） | 自动检测：先 HTTP，检测到 SPA 则回退到 JS |
| `true` | 强制 JS 渲染 |
| `false` | 仅 HTTP，跳过 JS |

### SPA 检测

在 `auto` 模式下，CRW 通过以下特征检测 SPA shell：
- 空的 `<body>` 或只有 `<div id="root">` 的最小内容
- 框架标记：React、Vue、Angular、Svelte
- 提示需要 JS 的 noscript 标签

检测到后，CRW 自动使用 JS 渲染器重试。

---

## LLM 结构化提取

使用 Claude 或 OpenAI 从网页中提取结构化 JSON。

### 设置

```bash
# 使用 Anthropic (Claude)
CRW_EXTRACTION__LLM__PROVIDER=anthropic \
CRW_EXTRACTION__LLM__API_KEY=sk-ant-... \
./target/release/crw-server

# 使用 OpenAI
CRW_EXTRACTION__LLM__PROVIDER=openai \
CRW_EXTRACTION__LLM__API_KEY=sk-... \
CRW_EXTRACTION__LLM__MODEL=gpt-4o \
./target/release/crw-server
```

### 使用方法

在抓取请求中发送 `jsonSchema` 并在 formats 中包含 `"json"`：

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com",
    "formats": ["json"],
    "jsonSchema": {
      "type": "object",
      "properties": {
        "title": {"type": "string"},
        "description": {"type": "string"}
      }
    }
  }'
```

LLM 会读取页面的 Markdown 内容，并返回与你的 Schema 匹配的结构化数据。

---

## 日志

CRW 使用 `RUST_LOG` 环境变量控制日志级别。

```bash
# Info 级别（默认）
RUST_LOG=info ./target/release/crw-server

# Debug 级别（详细）
RUST_LOG=debug ./target/release/crw-server

# 模块级别
RUST_LOG=crw_server=debug,crw_renderer=info ./target/release/crw-server
```

---

## 安全限制

CRW 实施以下限制以防止滥用：

| 限制 | 值 |
|:-----|:---|
| 最大响应体 | 10 MB |
| 最大爬取深度 | 10 |
| 每次爬取最大页面数 | 1000 |
| 允许的 URL 协议 | 仅 `http`、`https` |
| robots.txt | 默认遵守 |
