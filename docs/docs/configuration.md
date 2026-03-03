# Configuration

Configuration is loaded in layers: **defaults → config file → environment variables**.

## Config files

crw looks for configuration in this order:

1. `config.default.toml` (embedded defaults)
2. `config.local.toml` in the working directory
3. File specified by `CRW_CONFIG` environment variable

## Full reference

```toml
[server]
host = "0.0.0.0"
port = 3000
request_timeout_secs = 60

[renderer]
mode = "auto"               # auto | lightpanda | playwright | chrome | none
page_timeout_ms = 30000
pool_size = 4

[renderer.lightpanda]
ws_url = "ws://127.0.0.1:9222/"

# [renderer.playwright]
# ws_url = "ws://playwright:9222"

# [renderer.chrome]
# ws_url = "ws://chrome:9222"

[crawler]
max_concurrency = 10
requests_per_second = 10.0
respect_robots_txt = true
user_agent = "CRW/0.0.1"
default_max_depth = 2
default_max_pages = 100
job_ttl_secs = 3600
# proxy = "http://proxy:8080"

[extraction]
default_format = "markdown"
only_main_content = true

[extraction.llm]
provider = "anthropic"       # "anthropic" or "openai"
api_key = ""
model = "claude-sonnet-4-20250514"
max_tokens = 4096
# base_url = ""              # for OpenAI-compatible endpoints

[auth]
# api_keys = ["fc-key-1234"]
```

## Environment variables

Use the `CRW_` prefix with `__` as a nesting separator:

| Config | Environment Variable |
|--------|---------------------|
| `server.port` | `CRW_SERVER__PORT` |
| `server.host` | `CRW_SERVER__HOST` |
| `renderer.mode` | `CRW_RENDERER__MODE` |
| `crawler.max_concurrency` | `CRW_CRAWLER__MAX_CONCURRENCY` |
| `crawler.requests_per_second` | `CRW_CRAWLER__REQUESTS_PER_SECOND` |
| `extraction.llm.api_key` | `CRW_EXTRACTION__LLM__API_KEY` |
| `extraction.llm.provider` | `CRW_EXTRACTION__LLM__PROVIDER` |

## Renderer modes

| Mode | Description |
|------|-------------|
| `auto` | HTTP first, auto-detect SPAs, CDP fallback |
| `lightpanda` | Always use LightPanda via CDP |
| `playwright` | Always use Playwright via CDP |
| `chrome` | Always use Chrome via CDP |
| `none` | HTTP only, no JS rendering |

## Docker configuration

For Docker deployments, use `config.docker.toml` or environment variables:

```bash
docker run -p 3000:3000 \
  -e CRW_SERVER__PORT=3000 \
  -e CRW_RENDERER__MODE=lightpanda \
  -e CRW_EXTRACTION__LLM__API_KEY=sk-... \
  ghcr.io/us/crw:latest
```
