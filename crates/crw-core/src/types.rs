use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Supported output formats.
///
/// `"extract"` and `"llm-extract"` are accepted as aliases for `Json`
/// during deserialization (Firecrawl compatibility).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OutputFormat {
    Markdown,
    Html,
    RawHtml,
    PlainText,
    Links,
    Json,
}

impl<'de> Deserialize<'de> for OutputFormat {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "markdown" => Ok(OutputFormat::Markdown),
            "html" => Ok(OutputFormat::Html),
            "rawHtml" => Ok(OutputFormat::RawHtml),
            "plainText" => Ok(OutputFormat::PlainText),
            "links" => Ok(OutputFormat::Links),
            "json" | "extract" | "llm-extract" => Ok(OutputFormat::Json),
            other => Err(serde::de::Error::custom(format!(
                "Unknown format '{other}'. Valid formats: markdown, html, rawHtml, plainText, links, json \
                 (aliases: extract, llm-extract). Use formats: [\"json\"] with jsonSchema for structured extraction."
            ))),
        }
    }
}

/// Strategy for chunking text content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum ChunkStrategy {
    /// Split on sentence boundaries (.!?). Merges short chunks up to max_chars.
    #[serde(rename = "sentence")]
    Sentence {
        #[serde(default, alias = "maxChars")]
        max_chars: Option<usize>,
        #[serde(default, alias = "overlapChars")]
        overlap_chars: Option<usize>,
        #[serde(default)]
        dedupe: Option<bool>,
    },
    /// Split on a regex pattern.
    #[serde(rename = "regex")]
    Regex {
        pattern: String,
        #[serde(default, alias = "maxChars")]
        max_chars: Option<usize>,
        #[serde(default, alias = "overlapChars")]
        overlap_chars: Option<usize>,
        #[serde(default)]
        dedupe: Option<bool>,
    },
    /// Split on markdown headings (h1-h6).
    #[serde(rename = "topic")]
    Topic {
        #[serde(default, alias = "maxChars")]
        max_chars: Option<usize>,
        #[serde(default, alias = "overlapChars")]
        overlap_chars: Option<usize>,
        #[serde(default)]
        dedupe: Option<bool>,
    },
}

/// Filtering mode for ranked chunk retrieval.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FilterMode {
    Bm25,
    Cosine,
}

/// Per-request renderer override. Sibling to `renderJs` for finer control.
///
/// `Auto` is equivalent to omitting the field — uses the configured fallback chain.
/// Other variants hard-pin to a specific renderer with no fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RequestedRenderer {
    Auto,
    Lightpanda,
    Chrome,
    Playwright,
}

impl RequestedRenderer {
    /// Returns `Some(name)` for renderers that should be hard-pinned in dispatch.
    /// `Auto` returns `None` — equivalent to omitting the field.
    pub fn pinned_name(self) -> Option<&'static str> {
        match self {
            RequestedRenderer::Auto => None,
            RequestedRenderer::Lightpanda => Some("lightpanda"),
            RequestedRenderer::Chrome => Some("chrome"),
            RequestedRenderer::Playwright => Some("playwright"),
        }
    }
}

/// Firecrawl-compatible extraction options (used via `extract: { schema: {...} }`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractOptions {
    #[serde(default)]
    pub schema: Option<serde_json::Value>,
}

/// POST /v1/scrape request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrapeRequest {
    pub url: String,
    #[serde(default = "default_formats")]
    pub formats: Vec<OutputFormat>,
    #[serde(default = "default_true", alias = "only_main_content")]
    pub only_main_content: bool,
    /// null = auto-detect, true = force JS, false = skip JS
    #[serde(alias = "render_js")]
    pub render_js: Option<bool>,
    /// Milliseconds to wait after JS rendering.
    #[serde(alias = "wait_for")]
    pub wait_for: Option<u64>,
    #[serde(default, alias = "include_tags")]
    pub include_tags: Vec<String>,
    #[serde(default, alias = "exclude_tags")]
    pub exclude_tags: Vec<String>,
    #[serde(alias = "json_schema")]
    pub json_schema: Option<serde_json::Value>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// CSS selector to narrow content before extraction.
    #[serde(default, alias = "css_selector")]
    pub css_selector: Option<String>,
    /// XPath expression to narrow content before extraction.
    #[serde(default)]
    pub xpath: Option<String>,
    /// Strategy for chunking the extracted markdown.
    #[serde(default, alias = "chunk_strategy")]
    pub chunk_strategy: Option<ChunkStrategy>,
    /// Query string for BM25/cosine chunk filtering.
    #[serde(default)]
    pub query: Option<String>,
    /// Filtering algorithm to rank chunks against query.
    #[serde(default, alias = "filter_mode")]
    pub filter_mode: Option<FilterMode>,
    /// Number of top chunks to return (default: 5).
    #[serde(default)]
    pub top_k: Option<usize>,
    /// Per-request proxy URL (overrides global config).
    /// Supports HTTP, HTTPS, and SOCKS5
    /// (e.g. "http://proxy:8080" or "socks5://user:pass@proxy:1080").
    #[serde(default)]
    pub proxy: Option<String>,
    /// Override stealth mode for this request (None = use global config).
    #[serde(default)]
    pub stealth: Option<bool>,
    /// Unsupported Firecrawl parameter — captured to return a clear error.
    #[serde(default)]
    pub actions: Option<serde_json::Value>,
    /// Firecrawl-compatible `extract` object (e.g. `{ "schema": {...} }`).
    /// If `extract.schema` is set and `jsonSchema` is not, uses `extract.schema` as the schema.
    #[serde(default)]
    pub extract: Option<ExtractOptions>,
    /// Per-request LLM API key for structured extraction (BYOK).
    #[serde(default, alias = "llm_api_key")]
    pub llm_api_key: Option<String>,
    /// Per-request LLM provider override ("anthropic" or "openai").
    #[serde(default, alias = "llm_provider")]
    pub llm_provider: Option<String>,
    /// Per-request LLM model override.
    #[serde(default, alias = "llm_model")]
    pub llm_model: Option<String>,
    /// Pin this request to a specific renderer. `None` or `Auto` = use the
    /// configured chain. Hard-pin: pinned renderer failures surface as errors,
    /// no silent fallback to a different renderer or HTTP. Pinning a non-Auto
    /// value implies `renderJs:true` unless `renderJs:false` is set explicitly.
    #[serde(default)]
    pub renderer: Option<RequestedRenderer>,
}

fn default_formats() -> Vec<OutputFormat> {
    vec![OutputFormat::Markdown]
}

fn default_true() -> bool {
    true
}

/// Metadata about a scraped page.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageMetadata {
    pub title: Option<String>,
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub og_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub og_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub og_image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_url: Option<String>,
    #[serde(rename = "sourceURL")]
    pub source_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub status_code: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rendered_with: Option<String>,
    pub elapsed_ms: u64,
}

/// A single chunk with optional relevance score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkResult {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    pub index: usize,
}

/// Data returned for a single scraped page.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrapeData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markdown: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_html: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plain_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunks: Option<Vec<ChunkResult>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    /// Soft-failure / informational warnings collected through the render
    /// chain. Empty vec serializes as missing for backward compatibility.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
    /// Routing decision metadata (renderer chosen, failover chain).
    /// Surfaced for debug + UI; `None` for legacy paths.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub render_decision: Option<RenderDecision>,
    /// Credit cost attributed to this page (0 = not yet priced).
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub credit_cost: u32,
    pub metadata: PageMetadata,
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

/// Generic API response wrapper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T: Serialize> {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
            error_code: None,
            warning: None,
        }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(msg.into()),
            error_code: None,
            warning: None,
        }
    }

    pub fn err_with_code(msg: impl Into<String>, code: impl Into<String>) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(msg.into()),
            error_code: Some(code.into()),
            warning: None,
        }
    }
}

// ── Crawl types ──

/// POST /v1/crawl request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrawlRequest {
    pub url: String,
    pub max_depth: Option<u32>,
    #[serde(alias = "limit", alias = "max_pages")]
    pub max_pages: Option<u32>,
    #[serde(default = "default_formats")]
    pub formats: Vec<OutputFormat>,
    #[serde(default = "default_true")]
    pub only_main_content: bool,
    #[serde(default, alias = "json_schema")]
    pub json_schema: Option<serde_json::Value>,
    /// null = auto-detect (use global default), true = force JS, false = skip JS.
    /// Applies to every page fetched during the crawl.
    #[serde(default, alias = "render_js")]
    pub render_js: Option<bool>,
    /// Milliseconds to wait after JS rendering on each page.
    #[serde(default, alias = "wait_for")]
    pub wait_for: Option<u64>,
    /// Pin every page in this crawl to a specific renderer. See `ScrapeRequest::renderer`.
    #[serde(default)]
    pub renderer: Option<RequestedRenderer>,
}

/// Resolve the effective `render_js` decision from a per-request value and the
/// global default. Per-request always wins when set; otherwise fall back to the
/// default. `None` at both ends means "auto-detect".
///
/// Precedence table:
///
/// | request       | default       | effective    |
/// |---------------|---------------|--------------|
/// | `Some(true)`  | any           | `Some(true)` |
/// | `Some(false)` | any           | `Some(false)`|
/// | `None`        | `Some(true)`  | `Some(true)` |
/// | `None`        | `Some(false)` | `Some(false)`|
/// | `None`        | `None`        | `None`       |
pub fn resolve_render_js(request: Option<bool>, default: Option<bool>) -> Option<bool> {
    request.or(default)
}

/// Resolve the effective pinned renderer name from a per-request value.
///
/// Returns the renderer name (e.g. `"chrome"`) when a non-`Auto` renderer is pinned.
/// `None` and `Some(Auto)` both return `None` — meaning "use the configured chain".
pub fn resolve_pinned_renderer(req: Option<RequestedRenderer>) -> Option<&'static str> {
    req.and_then(|r| r.pinned_name())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_render_js_request_wins_true() {
        assert_eq!(resolve_render_js(Some(true), Some(false)), Some(true));
    }

    #[test]
    fn resolve_render_js_request_wins_false() {
        assert_eq!(resolve_render_js(Some(false), Some(true)), Some(false));
    }

    #[test]
    fn resolve_render_js_falls_back_to_default() {
        assert_eq!(resolve_render_js(None, Some(true)), Some(true));
        assert_eq!(resolve_render_js(None, Some(false)), Some(false));
    }

    #[test]
    fn resolve_render_js_both_none() {
        assert_eq!(resolve_render_js(None, None), None);
    }

    #[test]
    fn crawl_request_accepts_render_js_camel_case() {
        let json = serde_json::json!({
            "url": "https://example.com",
            "renderJs": true,
            "waitFor": 2000
        });
        let req: CrawlRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.render_js, Some(true));
        assert_eq!(req.wait_for, Some(2000));
    }

    #[test]
    fn crawl_request_accepts_render_js_snake_case() {
        let json = serde_json::json!({
            "url": "https://example.com",
            "render_js": false,
            "wait_for": 1500
        });
        let req: CrawlRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.render_js, Some(false));
        assert_eq!(req.wait_for, Some(1500));
    }

    #[test]
    fn crawl_request_render_fields_optional() {
        let json = serde_json::json!({ "url": "https://example.com" });
        let req: CrawlRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.render_js, None);
        assert_eq!(req.wait_for, None);
    }

    #[test]
    fn requested_renderer_deserializes_lowercase() {
        for (s, expected) in [
            ("\"auto\"", RequestedRenderer::Auto),
            ("\"lightpanda\"", RequestedRenderer::Lightpanda),
            ("\"chrome\"", RequestedRenderer::Chrome),
            ("\"playwright\"", RequestedRenderer::Playwright),
        ] {
            let parsed: RequestedRenderer = serde_json::from_str(s).unwrap();
            assert_eq!(parsed, expected, "input {s} should parse to {expected:?}");
        }
    }

    #[test]
    fn requested_renderer_rejects_unknown() {
        let result: Result<RequestedRenderer, _> = serde_json::from_str("\"firefox\"");
        assert!(
            result.is_err(),
            "unknown renderer should fail to deserialize"
        );
    }

    #[test]
    fn scrape_request_accepts_renderer_field() {
        let json = serde_json::json!({
            "url": "https://example.com",
            "renderer": "chrome"
        });
        let req: ScrapeRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.renderer, Some(RequestedRenderer::Chrome));
    }

    #[test]
    fn scrape_request_renderer_explicit_null() {
        let json = serde_json::json!({
            "url": "https://example.com",
            "renderer": null
        });
        let req: ScrapeRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.renderer, None);
    }

    #[test]
    fn scrape_request_renderer_omitted() {
        let json = serde_json::json!({ "url": "https://example.com" });
        let req: ScrapeRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.renderer, None);
    }

    #[test]
    fn crawl_request_accepts_renderer_field() {
        let json = serde_json::json!({
            "url": "https://example.com",
            "renderer": "lightpanda"
        });
        let req: CrawlRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.renderer, Some(RequestedRenderer::Lightpanda));
    }

    #[test]
    fn resolve_pinned_renderer_auto_returns_none() {
        assert_eq!(resolve_pinned_renderer(Some(RequestedRenderer::Auto)), None);
        assert_eq!(resolve_pinned_renderer(None), None);
    }

    #[test]
    fn resolve_pinned_renderer_chrome_returns_name() {
        assert_eq!(
            resolve_pinned_renderer(Some(RequestedRenderer::Chrome)),
            Some("chrome")
        );
        assert_eq!(
            resolve_pinned_renderer(Some(RequestedRenderer::Lightpanda)),
            Some("lightpanda")
        );
        assert_eq!(
            resolve_pinned_renderer(Some(RequestedRenderer::Playwright)),
            Some("playwright")
        );
    }
}

/// Status of an async crawl job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrawlStatus {
    #[serde(rename = "scraping")]
    InProgress,
    #[serde(rename = "completed")]
    Completed,
    #[serde(rename = "failed")]
    Failed,
}

/// GET /v1/crawl/:id response body.
/// Field names match Firecrawl API: status, total, completed, data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlState {
    #[serde(skip_serializing)]
    pub id: Uuid,
    pub success: bool,
    pub status: CrawlStatus,
    pub total: u32,
    pub completed: u32,
    pub data: Vec<ScrapeData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// POST /v1/crawl start response.
/// Matches Firecrawl format: { success: true, id: "..." }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlStartResponse {
    pub success: bool,
    pub id: String,
}

/// POST /v1/map request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapRequest {
    pub url: String,
    pub max_depth: Option<u32>,
    #[serde(default = "default_true")]
    pub use_sitemap: bool,
    /// Custom timeout in seconds (default: 120).
    #[serde(default)]
    pub timeout: Option<u64>,
}

/// POST /v1/map response data — the discovered links.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapData {
    pub links: Vec<String>,
}

/// POST /v1/map response body.
/// Standard envelope: { success: true, data: { links: [...] } }
pub type MapResponse = ApiResponse<MapData>;

// ── Render result ──

/// Closed enum of renderer kinds used in routing decisions and metrics.
/// Distinct from `RequestedRenderer` (user-facing input) — this is the
/// internal vocabulary for what actually executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RendererKind {
    Http,
    Lightpanda,
    Chrome,
}

impl RendererKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RendererKind::Http => "http",
            RendererKind::Lightpanda => "lightpanda",
            RendererKind::Chrome => "chrome",
        }
    }
}

/// Why and how a renderer was chosen for a given request. Surfaced in
/// `FetchResult.render_decision` and exposed to API callers behind a debug gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum RenderDecision {
    /// User pinned a specific renderer; auto-mode learning is bypassed.
    UserPinned { renderer: RendererKind },
    /// Auto mode used the configured default chain (no host preference yet).
    AutoDefault { chosen: RendererKind },
    /// Auto mode promoted a heavy renderer based on host preference.
    AutoPromoted {
        chosen: RendererKind,
        from: RendererKind,
        reason: String,
    },
    /// Auto mode skipped a renderer because its circuit breaker was open.
    BreakerSkipped {
        skipped: RendererKind,
        chosen: RendererKind,
    },
    /// Failover triggered after the initial choice failed.
    Failover {
        chain: Vec<RendererKind>,
        reason: FailoverErrorKind,
    },
}

/// Closed taxonomy of failure reasons that drive failover and host learning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FailoverErrorKind {
    /// LightPanda hydration / runtime exception (counts toward promotion).
    NextJsClientError,
    /// LightPanda returned an empty Next.js root shell.
    EmptyNextRoot,
    /// LightPanda timed out.
    LightpandaTimeout,
    /// LightPanda crashed or connection died.
    LightpandaCrash,
    /// Cloudflare challenge detected (combination markers).
    CloudflareChallenge,
    /// Generic placeholder / too-short content.
    PlaceholderContent,
    /// Network error during render.
    NetworkError,
    /// Other / unclassified failure (does NOT count for promotion).
    Other,
}

impl FailoverErrorKind {
    /// Strict failure predicate: only LightPanda-specific failures should
    /// drive host preference promotion. CF challenges and network errors
    /// are not LightPanda's fault.
    pub fn counts_for_promotion(&self) -> bool {
        matches!(
            self,
            FailoverErrorKind::NextJsClientError
                | FailoverErrorKind::EmptyNextRoot
                | FailoverErrorKind::LightpandaTimeout
                | FailoverErrorKind::LightpandaCrash
                | FailoverErrorKind::PlaceholderContent
        )
    }

    /// Stable camelCase identifier matching the JSON `serde` rendering.
    /// Used in user-facing warnings so the string a client sees in a
    /// `warnings[]` entry matches the `renderDecision.reason` field.
    pub fn as_str(&self) -> &'static str {
        match self {
            FailoverErrorKind::NextJsClientError => "nextJsClientError",
            FailoverErrorKind::EmptyNextRoot => "emptyNextRoot",
            FailoverErrorKind::LightpandaTimeout => "lightpandaTimeout",
            FailoverErrorKind::LightpandaCrash => "lightpandaCrash",
            FailoverErrorKind::CloudflareChallenge => "cloudflareChallenge",
            FailoverErrorKind::PlaceholderContent => "placeholderContent",
            FailoverErrorKind::NetworkError => "networkError",
            FailoverErrorKind::Other => "other",
        }
    }
}

/// Result of fetching + optionally rendering a page.
#[derive(Debug, Clone)]
pub struct FetchResult {
    pub url: String,
    pub status_code: u16,
    pub html: String,
    pub content_type: Option<String>,
    pub raw_bytes: Option<Vec<u8>>,
    pub rendered_with: Option<String>,
    pub elapsed_ms: u64,
    pub warning: Option<String>,
    /// Routing decision metadata. `None` for legacy / non-instrumented paths.
    pub render_decision: Option<RenderDecision>,
    /// Credit cost for this request (set by routing layer; 0 = not yet priced).
    pub credit_cost: u32,
    /// Soft-failure / informational warnings to surface to the caller.
    pub warnings: Vec<String>,
}
