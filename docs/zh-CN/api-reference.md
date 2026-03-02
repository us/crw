---
title: API 参考
layout: default
nav_order: 4
description: "CRW API 参考：scrape、crawl、map 端点，包含请求/响应结构和 curl 示例。"
parent: 首页
---

# API 参考
{: .no_toc }

所有端点、请求/响应结构和示例。
{: .fs-6 .fw-300 }

## 目录
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## 基础 URL

```
http://localhost:3000
```

配置 API 密钥后，所有 `/v1/*` 端点需要身份验证。`/health` 端点始终公开。

---

## 健康检查

```
GET /health
```

返回服务器状态和渲染器可用性。无需身份验证。

### 示例

```bash
curl http://localhost:3000/health
```

### 响应

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

| 字段 | 类型 | 描述 |
|:-----|:-----|:-----|
| `status` | string | 服务器运行时始终为 `"ok"` |
| `version` | string | 服务器版本 |
| `renderers` | object | 渲染器名称 → 可用性映射 |
| `active_crawl_jobs` | number | 当前正在运行的爬取任务数 |

---

## 抓取（Scrape）

```
POST /v1/scrape
```

抓取单个 URL 并以一种或多种格式提取内容。

### 请求体

| 字段 | 类型 | 必需 | 默认值 | 描述 |
|:-----|:-----|:-----|:-------|:-----|
| `url` | string | **是** | — | 要抓取的 URL（仅 `http`/`https`） |
| `formats` | string[] | 否 | `["markdown"]` | 输出格式（见下方） |
| `onlyMainContent` | boolean | 否 | `true` | 去除导航栏、页脚、侧边栏 |
| `renderJs` | boolean\|null | 否 | `null` | `null`=自动, `true`=强制 JS, `false`=仅 HTTP |
| `waitFor` | number | 否 | — | JS 渲染后等待的毫秒数 |
| `includeTags` | string[] | 否 | `[]` | CSS 选择器 — 仅包含匹配元素 |
| `excludeTags` | string[] | 否 | `[]` | CSS 选择器 — 移除匹配元素 |
| `headers` | object | 否 | `{}` | 自定义 HTTP 请求头 |
| `jsonSchema` | object | 否 | — | LLM 结构化提取的 JSON Schema |

**输出格式：**

| 格式 | 描述 |
|:-----|:-----|
| `markdown` | 清理后的 HTML 转换为 Markdown |
| `html` | 清理后的 HTML（移除脚本、样式、广告） |
| `rawHtml` | 原始 HTML（不做处理） |
| `plainText` | 纯文本内容，无标记 |
| `links` | 页面上所有链接的数组 |
| `json` | LLM 提取的结构化数据（需要 `jsonSchema` + LLM 配置） |

{: .note }
也接受蛇形命名别名：`only_main_content`、`render_js`、`wait_for`、`include_tags`、`exclude_tags`、`json_schema`。

### 响应体

```json
{
  "success": true,
  "data": {
    "markdown": "字符串或 null",
    "html": "字符串或 null",
    "rawHtml": "字符串或 null",
    "plainText": "字符串或 null",
    "links": ["字符串"] 或 null,
    "json": {} 或 null,
    "metadata": {
      "title": "字符串或 null",
      "description": "字符串或 null",
      "ogTitle": "字符串或 null",
      "ogDescription": "字符串或 null",
      "ogImage": "字符串或 null",
      "canonicalUrl": "字符串或 null",
      "sourceURL": "字符串（始终存在）",
      "language": "字符串或 null",
      "statusCode": 200,
      "renderedWith": "字符串或 null",
      "elapsedMs": 32
    }
  }
}
```

仅返回请求的格式，其他为 `null`。

### 示例

**基本抓取：**

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

**多种格式：**

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com",
    "formats": ["markdown", "html", "links"]
  }'
```

**使用 CSS 选择器：**

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com",
    "excludeTags": ["nav", "footer", ".sidebar"]
  }'
```

**强制 JS 渲染：**

```bash
curl -X POST http://localhost:3000/v1/scrape \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://spa-app.example.com",
    "renderJs": true,
    "waitFor": 2000
  }'
```

---

## 爬取（Crawl）

### 启动爬取

```
POST /v1/crawl
```

启动异步 BFS 爬取。立即返回任务 ID。

#### 请求体

| 字段 | 类型 | 必需 | 默认值 | 描述 |
|:-----|:-----|:-----|:-------|:-----|
| `url` | string | **是** | — | 起始 URL（仅 `http`/`https`） |
| `maxDepth` | number | 否 | `2` | 最大链接跟踪深度 |
| `maxPages` | number | 否 | `100` | 最大抓取页面数 |
| `formats` | string[] | 否 | `["markdown"]` | 每页的输出格式 |
| `onlyMainContent` | boolean | 否 | `true` | 去除每页的模板内容 |

#### 示例

```bash
curl -X POST http://localhost:3000/v1/crawl \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://example.com",
    "maxDepth": 1,
    "maxPages": 2
  }'
```

#### 响应

```json
{
  "success": true,
  "id": "a4c03342-ab36-4df6-9e15-7ecffc9f8b3a"
}
```

### 查询爬取状态

```
GET /v1/crawl/:id
```

轮询爬取任务的状态和结果。

#### 示例

```bash
curl http://localhost:3000/v1/crawl/a4c03342-ab36-4df6-9e15-7ecffc9f8b3a
```

#### 响应

```json
{
  "status": "completed",
  "total": 1,
  "completed": 1,
  "data": [
    {
      "markdown": "# Example Domain\nThis domain is for use in documentation examples without needing permission. Avoid use in operations.\n[Learn more](https://iana.org/domains/example)",
      "metadata": {
        "title": "Example Domain",
        "sourceURL": "https://example.com",
        "statusCode": 200,
        "elapsedMs": 12
      }
    }
  ]
}
```

#### 状态值

| 状态 | 描述 |
|:-----|:-----|
| `scraping` | 爬取进行中 |
| `completed` | 所有页面抓取成功 |
| `failed` | 爬取遇到致命错误 |

#### 响应字段

| 字段 | 类型 | 描述 |
|:-----|:-----|:-----|
| `status` | string | 当前爬取状态 |
| `total` | number | 已发现的 URL 总数 |
| `completed` | number | 已抓取的页面数 |
| `data` | array | 抓取结果数组（与 `/v1/scrape` 数据格式相同） |
| `error` | string\|null | 失败时的错误信息 |

{: .note }
已完成的爬取任务会在配置的 TTL 后自动清理（默认：1 小时）。

---

## 站点地图（Map）

```
POST /v1/map
```

通过爬取和站点地图解析发现网站上的所有 URL。

### 请求体

| 字段 | 类型 | 必需 | 默认值 | 描述 |
|:-----|:-----|:-----|:-------|:-----|
| `url` | string | **是** | — | 要发现链接的 URL |
| `maxDepth` | number | 否 | `2` | 最大发现深度 |
| `useSitemap` | boolean | 否 | `true` | 同时读取 sitemap.xml |

### 示例

```bash
curl -X POST http://localhost:3000/v1/map \
  -H "Content-Type: application/json" \
  -d '{"url": "https://example.com"}'
```

### 响应

```json
{
  "success": true,
  "links": [
    "https://example.com"
  ]
}
```

---

## 错误响应

所有错误返回相同格式：

```json
{
  "success": false,
  "error": "可读的错误信息"
}
```

### HTTP 状态码

| 状态码 | 含义 | 触发条件 |
|:-------|:-----|:---------|
| `200` | 成功 | 请求成功 |
| `400` | 错误请求 | URL 无效、缺少必填字段、非 http(s) 协议 |
| `401` | 未授权 | 缺少或无效的 Bearer Token |
| `404` | 未找到 | 爬取任务 ID 不存在 |
| `422` | 无法处理 | LLM 提取失败 |
| `502` | 网关错误 | 目标网站返回错误 |
| `504` | 网关超时 | 请求超时 |
| `500` | 服务器内部错误 | 意外的服务器错误 |
