---
title: 快速开始
layout: default
nav_order: 2
description: "一分钟内安装并运行 CRW 网页抓取工具。从源码构建或使用 Docker。"
parent: 首页
---

# 快速开始
{: .no_toc }

一分钟内启动 CRW。
{: .fs-6 .fw-300 }

## 目录
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## 系统要求

- **Rust 1.83+**（从源码构建）
- **Docker**（可选，用于容器化部署）

## 从源码安装

```bash
git clone https://github.com/us/crw.git
cd crw
```

### 仅 HTTP（最快构建）

不包含 JS 渲染。适用于静态网站和 API。

```bash
cargo build --release --bin crw-server
```

### 包含 JS 渲染

添加 CDP（Chrome DevTools Protocol）支持，用于渲染 SPA 单页应用。

```bash
cargo build --release --bin crw-server --features crw-server/cdp
```

### MCP 服务器

用于 Claude Code / Claude Desktop 集成。

```bash
cargo build --release --bin crw-mcp
```

### 生成的二进制文件

| 二进制文件 | 路径 | 描述 |
|-----------|------|------|
| `crw-server` | `target/release/crw-server` | API 服务器 |
| `crw-mcp` | `target/release/crw-mcp` | LLM 工具的 MCP 服务器 |

## 使用 Docker 安装

```bash
git clone https://github.com/us/crw.git
cd crw
docker compose up
```

这将启动：
- **crw**（端口 `3000`）— 启用 CDP 的 API 服务器
- **lightpanda**（端口 `9222`）— JS 渲染 sidecar

## 运行服务器

```bash
./target/release/crw-server
```

你将看到：

```
INFO crw_server: Starting CRW on 0.0.0.0:3000
INFO crw_server: Renderer mode: auto
INFO crw_server: CRW ready at http://0.0.0.0:3000
```

## 验证运行

### 健康检查

```bash
curl http://localhost:3000/health
```

```json
{
  "status": "ok",
  "version": "0.1.0",
  "renderers": {
    "http": true
  },
  "active_crawl_jobs": 0
}
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
    "markdown": "# Example Domain\nThis domain is for use in documentation examples without needing permission. Avoid use in operations.\n[Learn more](https://iana.org/domains/example)",
    "metadata": {
      "title": "Example Domain",
      "sourceURL": "https://example.com",
      "language": "en",
      "statusCode": 200,
      "elapsedMs": 32
    }
  }
}
```

## 下一步

- [配置说明]({% link zh-CN/configuration.md %}) — 自定义端口、渲染器、速率限制
- [API 参考]({% link zh-CN/api-reference.md %}) — 所有端点及示例
- [MCP 服务器]({% link zh-CN/mcp-server.md %}) — 在 Claude Code 中使用 CRW
- [Docker 部署]({% link zh-CN/docker.md %}) — 生产环境部署
