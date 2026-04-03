# CRW Roadmap

## Hard / Future Items

These features require significant effort or external dependencies and are planned for future releases.

### PDF / DOCX Parsing
- Extract text content from PDF and DOCX files during scrape/crawl
- Potential crates: `lopdf`, `pdf-extract`, `docx-rs`
- Requires content-type detection and format-specific extraction pipeline

### Screenshot Capture
- Capture page screenshots via CDP `Page.captureScreenshot`
- Return as base64 or stored image URL
- Only available when CDP renderer is enabled

### /search Endpoint
- Full-text search across crawled content
- Requires external search backend integration
- Alternative: built-in index with tantivy

### Prometheus /metrics Endpoint
- Expose request counts, latencies, crawl job stats
- Use `metrics` + `metrics-exporter-prometheus` crates
- Endpoint: `GET /metrics`

### IP-based Rate Limiting
- Per-IP request throttling using token bucket or sliding window
- Use `tower` middleware or `governor` crate
- Configurable limits per endpoint

---

## Scalability & Production Readiness

These items address the current system's inability to handle high-load scenarios (tested: system crashes under ~10K+ concurrent requests due to unbounded queueing and in-memory state).

### Request Queue Limiting (Critical)
- **Problem:** Axum accepts unlimited concurrent requests, no backpressure — causes OOM under load
- Add `tower::buffer::Buffer` or `tower::limit::ConcurrencyLimit` middleware
- Return `503 Service Unavailable` with `Retry-After` header when queue is full
- Configurable max queue size via `config.toml`

### Global Rate Limiting (Critical)
- **Problem:** Rate limiting is per-crawl only (10 RPS), no global limit across all operations
- Add `tower-governor` middleware for global RPS limiting
- Separate limits for `/v1/scrape`, `/v1/crawl`, `/v1/map` endpoints
- Configurable via `[server.rate_limit]` in config

### External State Storage
- **Problem:** All crawl state (HTML + metadata) stored in memory — 10 crawls × 10MB = 100MB+, no persistence
- Move crawl job state to Redis or SQLite
- Benefits: survives restarts, enables horizontal scaling, bounded memory
- Keep in-memory as default for dev, external store for production

### CDP Connection Pooling
- **Problem:** Every JS-rendered page opens a fresh WebSocket connection to LightPanda/Chrome (10s connect timeout overhead)
- Implement persistent WebSocket connection pool with configurable `pool_size`
- Reuse connections across requests, health-check idle connections
- Fall back to per-request connections if pool exhausted

### Circuit Breaker
- **Problem:** If LightPanda/CDP renderer goes down, requests hang until timeout (30s)
- Add circuit breaker pattern: after N consecutive failures, stop trying CDP for a cooldown period
- Fast-fail with HTTP-only fallback instead of waiting for timeout
- Use `tower::retry` with backoff or custom implementation

### Horizontal Scaling
- **Problem:** Single-instance architecture, no load distribution
- Stateless request handling (with external state storage) enables multiple instances
- Add health check endpoint improvements for load balancer integration
- Document deployment with nginx/HAProxy + multiple CRW instances
- Kubernetes / Docker Compose scaling guide

### Observability & Monitoring
- Prometheus metrics (see above) + Grafana dashboard templates
- Structured logging with request IDs for tracing
- Alert thresholds: queue depth, memory usage, error rates, CDP availability
- OpenTelemetry integration for distributed tracing

### Graceful Shutdown & Backpressure
- **Problem:** No graceful shutdown — in-flight requests lost on restart
- Drain active connections before shutdown (tokio signal handling)
- Finish in-progress crawl pages, persist state, then exit
- Configurable drain timeout

### Benchmarks & Load Testing
- Add `k6` or `wrk` load test scripts to repo
- Establish baseline: max RPS, p50/p95/p99 latencies, memory ceiling
- CI pipeline for regression testing against performance baselines
- Document capacity planning guidelines per hardware tier
