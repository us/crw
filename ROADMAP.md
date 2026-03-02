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
- Requires external search API integration (e.g., SearXNG, Brave Search API)
- Alternative: built-in index with tantivy

### Prometheus /metrics Endpoint
- Expose request counts, latencies, crawl job stats
- Use `metrics` + `metrics-exporter-prometheus` crates
- Endpoint: `GET /metrics`

### IP-based Rate Limiting
- Per-IP request throttling using token bucket or sliding window
- Use `tower` middleware or `governor` crate
- Configurable limits per endpoint
