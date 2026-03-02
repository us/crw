---
title: MCP 服务器
layout: default
nav_order: 5
description: "在 Claude Code、Claude Desktop、Cursor、Windsurf、Cline、Continue.dev 和 OpenAI Codex 中通过 MCP 使用 CRW 网页抓取工具。"
parent: 首页
---

# MCP 服务器
{: .no_toc }

在任何支持 MCP 的 AI 编程助手中使用 CRW 作为网页抓取工具。
{: .fs-6 .fw-300 }

## 目录
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## 什么是 MCP？

[MCP（模型上下文协议）](https://modelcontextprotocol.io)是一个开放标准，允许 AI 助手使用外部工具。CRW 内置 MCP 服务器，为任何兼容 MCP 的客户端提供 4 个网页抓取工具。

## 两种传输方式

CRW 支持**两种方式**连接 MCP 客户端：

| 传输方式 | 设置 | 要求 |
|:---------|:-----|:-----|
| **HTTP**（推荐） | 一行命令，无需额外二进制 | `crw-server` 运行中 |
| **Stdio** | 需要独立二进制（`crw-mcp`） | `crw-server` 运行中 + `crw-mcp` 二进制 |

### 方式一：流式 HTTP（推荐）

`crw-server` 内置 `/mcp` 端点。无需额外二进制 — 只需将 MCP 客户端指向 URL：

```bash
claude mcp add --transport http crw http://localhost:3000/mcp
```

就这么简单。无需构建，无需指定二进制路径。

### 方式二：Stdio 二进制

构建独立的 MCP 二进制文件：

```bash
cargo build --release --bin crw-mcp
```

二进制文件位于 `target/release/crw-mcp`（约 4 MB）。它**不依赖** CRW 的内部 crate — 是一个纯粹的 JSON-RPC 2.0 stdio 代理。

{: .warning }
使用 MCP 工具前请确保 `crw-server` 正在运行。两种传输方式都会将请求转发到 HTTP API。

## 可用工具

连接后，以下工具将出现在你的 AI 助手中：

| 工具 | 描述 | HTTP 端点 |
|:-----|:-----|:----------|
| `crw_scrape` | 抓取 URL → Markdown、HTML、链接 | `POST /v1/scrape` |
| `crw_crawl` | 启动异步爬取 → 返回任务 ID | `POST /v1/crawl` |
| `crw_check_crawl_status` | 轮询爬取状态并获取结果 | `GET /v1/crawl/:id` |
| `crw_map` | 发现网站上的所有 URL | `POST /v1/map` |

### 工具参数

**crw_scrape：**

| 参数 | 类型 | 必需 | 描述 |
|:-----|:-----|:-----|:-----|
| `url` | string | **是** | 要抓取的 URL |
| `formats` | string[] | 否 | `markdown`、`html`、`links` |
| `onlyMainContent` | boolean | 否 | 去除导航/页脚（默认：true） |
| `includeTags` | string[] | 否 | 要保留的 CSS 选择器 |
| `excludeTags` | string[] | 否 | 要移除的 CSS 选择器 |

**crw_crawl：**

| 参数 | 类型 | 必需 | 描述 |
|:-----|:-----|:-----|:-----|
| `url` | string | **是** | 起始 URL |
| `maxDepth` | integer | 否 | 最大爬取深度（默认：2） |
| `maxPages` | integer | 否 | 最大页面数（默认：10） |

**crw_check_crawl_status：**

| 参数 | 类型 | 必需 | 描述 |
|:-----|:-----|:-----|:-----|
| `id` | string | **是** | `crw_crawl` 返回的任务 ID |

**crw_map：**

| 参数 | 类型 | 必需 | 描述 |
|:-----|:-----|:-----|:-----|
| `url` | string | **是** | 要映射的 URL |
| `maxDepth` | integer | 否 | 发现深度（默认：2） |
| `useSitemap` | boolean | 否 | 读取 sitemap.xml（默认：true） |

## MCP 环境变量

| 变量 | 默认值 | 描述 |
|:-----|:-------|:-----|
| `CRW_API_URL` | `http://localhost:3000` | CRW 服务器 URL |
| `CRW_API_KEY` | — | Bearer Token（启用认证时） |
| `RUST_LOG` | `crw_mcp=info` | 日志级别（日志输出到 stderr） |

---

## 各平台设置指南

### Claude Code

**HTTP 传输（推荐）：**

```bash
claude mcp add --transport http crw http://localhost:3000/mcp
```

**Stdio 传输：**

```bash
claude mcp add crw -- /absolute/path/to/crw-mcp
```

带环境变量：

```bash
claude mcp add --env CRW_API_URL=http://localhost:3000 crw -- /absolute/path/to/crw-mcp
```

或手动编辑 `~/.claude.json`：

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

项目级配置请在项目根目录创建 `.mcp.json`，格式相同。

添加后重启 Claude Code。

{: .tip }
使用 `claude mcp list` 验证 CRW 已注册，使用 `claude mcp remove crw` 卸载。

---

### Claude Desktop

编辑对应操作系统的配置文件：

| 操作系统 | 路径 |
|:---------|:-----|
| macOS | `~/Library/Application Support/Claude/claude_desktop_config.json` |
| Windows | `%APPDATA%\Claude\claude_desktop_config.json` |
| Linux | `~/.config/Claude/claude_desktop_config.json` |

添加：

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

完全退出并重启 Claude Desktop。

---

### Cursor

创建或编辑 `~/.cursor/mcp.json`（全局）或 `.cursor/mcp.json`（项目级）：

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

或使用 GUI：**Settings → Developer → MCP Tools → Add Custom MCP**。

Cursor 支持 stdio 和流式 HTTP 传输。

---

### Windsurf

编辑 `~/.codeium/windsurf/mcp_config.json`：

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

或使用 GUI：**Windsurf Settings → Cascade → MCP Servers**。

{: .note }
Windsurf 所有 MCP 服务器总共限制 100 个工具。CRW 仅使用 4 个。

---

### Cline（VS Code 扩展）

配置文件路径取决于操作系统：

| 操作系统 | 路径 |
|:---------|:-----|
| macOS | `~/Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json` |
| Windows | `%APPDATA%/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json` |
| Linux | `~/.config/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json` |

```json
{
  "mcpServers": {
    "crw": {
      "command": "/absolute/path/to/crw-mcp",
      "env": {
        "CRW_API_URL": "http://localhost:3000"
      },
      "alwaysAllow": ["crw_scrape", "crw_map"],
      "disabled": false
    }
  }
}
```

或使用 GUI：点击 Cline 顶部栏的 **MCP Servers** 图标 → Configure → "Configure MCP Servers"。

{: .tip }
为你信任的工具设置 `alwaysAllow`，可跳过每次使用时的审批提示。

---

### Continue.dev（VS Code / JetBrains）

编辑 `~/.continue/config.yaml`：

```yaml
mcpServers:
  - name: crw
    command: /absolute/path/to/crw-mcp
    env:
      CRW_API_URL: http://localhost:3000
```

或在项目中放置 JSON 文件 `.continue/mcpServers/crw.json`：

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

{: .note }
MCP 工具仅在 Continue 的 **Agent 模式**下工作，普通聊天模式不可用。

---

### OpenAI Codex CLI

编辑 `~/.codex/config.toml`：

```toml
[mcp_servers.crw]
command = "/absolute/path/to/crw-mcp"

[mcp_servers.crw.env]
CRW_API_URL = "http://localhost:3000"
```

或使用 CLI：

```bash
codex mcp add crw -- /absolute/path/to/crw-mcp
```

---

## 各平台对比

| 平台 | 配置格式 | 配置路径 | 一行命令 |
|:-----|:---------|:---------|:---------|
| Claude Code | JSON | `~/.claude.json` | `claude mcp add --transport http crw http://localhost:3000/mcp` |
| Claude Desktop | JSON | 因操作系统而异（见上方） | — |
| Cursor | JSON | `~/.cursor/mcp.json` | — |
| Windsurf | JSON | `~/.codeium/windsurf/mcp_config.json` | — |
| Cline | JSON | VS Code globalStorage | — |
| Continue.dev | YAML | `~/.continue/config.yaml` | — |
| OpenAI Codex | TOML | `~/.codex/config.toml` | `codex mcp add crw -- /path/to/crw-mcp` |

---

## 启用身份验证

如果 CRW 服务器启用了身份验证，在上述任何配置中添加 `CRW_API_KEY` 环境变量：

```json
{
  "mcpServers": {
    "crw": {
      "command": "/absolute/path/to/crw-mcp",
      "env": {
        "CRW_API_URL": "http://localhost:3000",
        "CRW_API_KEY": "your-api-key"
      }
    }
  }
}
```

---

## 验证安装

直接测试 MCP 服务器：

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"test"},"protocolVersion":"2024-11-05"}}' \
  | ./target/release/crw-mcp 2>/dev/null
```

预期输出：

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "protocolVersion": "2024-11-05",
    "capabilities": {"tools": {}},
    "serverInfo": {"name": "crw-mcp", "version": "0.1.0"}
  }
}
```

## 工作原理

**HTTP 传输（推荐）：**

```
AI 助手 (Claude, Cursor, Codex, ...)
    ↓ HTTP POST (JSON-RPC 2.0)
  crw-server /mcp 端点 (localhost:3000)
    ↓ 直接函数调用
  网页
```

**Stdio 传输：**

```
AI 助手 (Claude, Cursor, Codex, ...)
    ↓ stdin (JSON-RPC 2.0)
  crw-mcp 二进制
    ↓ HTTP (POST/GET)
  crw-server (localhost:3000)
    ↓
  网页
```

HTTP 传输直接调用内部函数，零额外开销。Stdio 传输是纯 JSON 代理，可与任何兼容 Firecrawl 的 API 后端配合使用。
