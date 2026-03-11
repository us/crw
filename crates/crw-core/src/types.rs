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
                "Unknown format '{other}'. Valid formats: markdown, html, rawHtml, plainText, links, json. \
                 Use formats: [\"json\"] with jsonSchema for structured extraction."
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
    /// Per-request HTTP proxy URL (overrides global config).
    #[serde(default)]
    pub proxy: Option<String>,
    /// Override stealth mode for this request (None = use global config).
    #[serde(default)]
    pub stealth: Option<bool>,
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
    pub metadata: PageMetadata,
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
    pub warning: Option<String>,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
            warning: None,
        }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(msg.into()),
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

/// POST /v1/map response body.
/// Matches Firecrawl format: { success: true, links: [...] }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapResponse {
    pub success: bool,
    pub links: Vec<String>,
}

// ── Render result ──

/// Result of fetching + optionally rendering a page.
#[derive(Debug, Clone)]
pub struct FetchResult {
    pub url: String,
    pub status_code: u16,
    pub html: String,
    pub rendered_with: Option<String>,
    pub elapsed_ms: u64,
    pub warning: Option<String>,
}
