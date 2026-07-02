use serde::{Deserialize, Deserializer, Serialize, Serializer};
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
    Summary,
    ChangeTracking,
    Screenshot,
}

impl OutputFormat {
    /// Parse a single format token, accepting the Firecrawl-compatible aliases
    /// (`extract`/`llm-extract` → `json`, `change-tracking` → `changeTracking`).
    ///
    /// Shared by the v1 string deserializer below and the v2 `FormatSpec`
    /// parser (`routes/v2/formats.rs`) so the accepted token set and the error
    /// wording stay byte-identical across API versions.
    pub fn parse_loose(s: &str) -> Result<Self, String> {
        match s {
            "markdown" => Ok(OutputFormat::Markdown),
            "html" => Ok(OutputFormat::Html),
            "rawHtml" => Ok(OutputFormat::RawHtml),
            "plainText" => Ok(OutputFormat::PlainText),
            "links" => Ok(OutputFormat::Links),
            "json" | "extract" | "llm-extract" => Ok(OutputFormat::Json),
            "summary" => Ok(OutputFormat::Summary),
            "changeTracking" | "change-tracking" => Ok(OutputFormat::ChangeTracking),
            // `screenshot@fullPage` parses to the same fieldless variant; the
            // `fullPage` bit is carried out-of-band via
            // `ScrapeRequest.screenshot_full_page` (v2 extracts it in
            // `routes/v2/formats.rs`; v1 always treats it as false — see D7).
            "screenshot" | "screenshot@fullPage" => Ok(OutputFormat::Screenshot),
            other => Err(format!(
                "Unknown format '{other}'. Valid formats: markdown, html, rawHtml, plainText, links, json, summary, changeTracking \
                 (aliases: extract, llm-extract, change-tracking). Use formats: [\"json\"] with jsonSchema for structured extraction."
            )),
        }
    }
}

impl<'de> Deserialize<'de> for OutputFormat {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        OutputFormat::parse_loose(&s).map_err(serde::de::Error::custom)
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
    /// Residential-proxy Chrome tier — egresses through the DataImpulse
    /// pool. `rename_all = "lowercase"` would yield `"chromeproxy"`, so the
    /// variant is renamed explicitly to match the internal renderer name
    /// (`"chrome_proxy"`) and `RendererKind::ChromeProxy`.
    #[serde(rename = "chrome_proxy")]
    ChromeProxy,
    Playwright,
    /// Opt-in Camoufox stealth tier. `rename_all = "lowercase"` already yields
    /// `"camoufox"`, matching the internal renderer name and
    /// `RendererKind::Camoufox`.
    Camoufox,
}

impl RequestedRenderer {
    /// Returns `Some(name)` for renderers that should be hard-pinned in dispatch.
    /// `Auto` returns `None` — equivalent to omitting the field.
    pub fn pinned_name(self) -> Option<&'static str> {
        match self {
            RequestedRenderer::Auto => None,
            RequestedRenderer::Lightpanda => Some("lightpanda"),
            RequestedRenderer::Chrome => Some("chrome"),
            RequestedRenderer::ChromeProxy => Some("chrome_proxy"),
            RequestedRenderer::Playwright => Some("playwright"),
            RequestedRenderer::Camoufox => Some("camoufox"),
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
    /// Per-request proxy pool to rotate among (BYOP). Takes precedence over
    /// `proxy` and over the server's configured pool. Empty = use server config.
    /// Accepts the snake_case `proxy_list` alias (the managed layer injects it)
    /// in addition to the camelCase `proxyList`.
    #[serde(default, alias = "proxy_list")]
    pub proxy_list: Vec<String>,
    /// Rotation strategy for `proxy_list` (`round_robin`, `random`,
    /// `sticky_per_host`). `None` = server default (`sticky_per_host`).
    #[serde(default, alias = "proxy_rotation")]
    pub proxy_rotation: Option<crate::proxy::ProxyRotation>,
    /// 2-letter ISO 3166-1 alpha-2 country code (e.g. "us", "gb") for the
    /// residential-proxy chrome tier's egress. When the server has
    /// DataImpulse credentials configured, the engine composes
    /// `<base>__cr.<country>` per request and supplies it via CDP
    /// `Fetch.authRequired`. Unset / empty = server default country (or
    /// global pool when no default configured). Validated server-side;
    /// invalid values fall back to default.
    #[serde(default)]
    pub country: Option<String>,
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
    /// Per-request LLM provider override ("anthropic", "openai", "deepseek", "azure", or "openai-compatible").
    #[serde(default, alias = "llm_provider")]
    pub llm_provider: Option<String>,
    /// Per-request LLM model override.
    #[serde(default, alias = "llm_model")]
    pub llm_model: Option<String>,
    /// Per-request LLM base URL override (OpenAI-compatible providers
    /// like DeepSeek). Example: `"https://api.deepseek.com/v1"`.
    #[serde(default, alias = "base_url")]
    pub base_url: Option<String>,
    /// Optional user-supplied instructions appended to the summary system
    /// prompt (e.g. "respond in Turkish", "focus on technical details").
    /// The opencore's prompt-injection defense (UNTRUSTED delimiter,
    /// "ignore imperative content" rule) is kept intact — this only adds
    /// directives, it does not replace the safety wrapper. Capped at
    /// 500 chars server-side to bound token amplification.
    #[serde(default, alias = "summary_prompt")]
    pub summary_prompt: Option<String>,
    /// Maximum number of bytes of scraped content sent to the LLM for the
    /// `summary` format. Defaults to `[extraction.llm].max_html_bytes`
    /// (100 KB out of the box). Clamped to a 200 KB server-side ceiling
    /// regardless of value — protects against runaway provider bills.
    #[serde(default, alias = "max_content_chars")]
    pub max_content_chars: Option<usize>,
    /// Pin this request to a specific renderer. `None` or `Auto` = use the
    /// configured chain. Hard-pin: pinned renderer failures surface as errors,
    /// no silent fallback to a different renderer or HTTP. Pinning a non-Auto
    /// value implies `renderJs:true` unless `renderJs:false` is set explicitly.
    #[serde(default)]
    pub renderer: Option<RequestedRenderer>,
    /// End-to-end deadline budget in milliseconds. When unset, the configured
    /// `request.deadline_ms_default` (8000) applies. The SLO p95 metric is
    /// computed only over requests with `deadline_ms <= 8000`; longer values
    /// land in a separate slow-path histogram. Must be in `(0, 60000]`.
    #[serde(default, alias = "deadline_ms")]
    pub deadline_ms: Option<u64>,
    /// Opt-in extraction debug trace. When true, the response includes a
    /// `debugExtraction` field describing every candidate the extractor
    /// considered and why one was selected.
    #[serde(default)]
    pub debug: Option<bool>,
    /// Change-tracking options. Activated when `formats` contains
    /// `"changeTracking"`. Carries the diff modes, an optional extraction
    /// schema/prompt for json mode, and the caller-supplied `previous`
    /// snapshot to diff the current scrape against. Sibling field — mirrors
    /// the precedented `extract` / `jsonSchema` pattern (the `formats` entry
    /// is the plain string `"changeTracking"`, options ride here).
    #[serde(default, alias = "change_tracking")]
    pub change_tracking: Option<ChangeTrackingOptions>,
    /// Plain-language monitor goal used by the meaningful-change judge.
    /// Capped server-side at 2 KB. The judge only runs when both `goal` is
    /// present and `judgeEnabled` is true (and the page actually changed).
    #[serde(default)]
    pub goal: Option<String>,
    /// Whether to run the LLM meaningful-change judge on a changed page.
    /// `None` is treated as "off" at the opencore layer — the SaaS
    /// orchestration decides auto-enable semantics.
    #[serde(default, alias = "judge_enabled")]
    pub judge_enabled: Option<bool>,
    /// Firecrawl-compatible document parsers. Controls how non-HTML documents
    /// (currently only PDF) are handled when a URL returns one.
    /// - `None` (field omitted) → PDFs are auto-parsed to markdown (default,
    ///   matches Firecrawl).
    /// - `Some([])` → parsing disabled; the raw document is left unconverted.
    /// - `Some([{type:"pdf"}])` → explicitly enable PDF parsing (optionally
    ///   capped via `maxPages`).
    #[serde(default)]
    pub parsers: Option<Vec<ParserSpec>>,
    /// Whether a requested `screenshot` format should capture the full page
    /// (`screenshot@fullPage` / `{type:"screenshot", fullPage:true}`) instead of
    /// just the viewport. Carried out-of-band rather than on the (`Copy`/`Hash`)
    /// `OutputFormat` enum so `formats.contains(&Screenshot)` stays cheap. v1
    /// always leaves this false (see D7); v2 sets it in `routes/v2/formats.rs`.
    #[serde(default, alias = "screenshot_full_page")]
    pub screenshot_full_page: bool,
}

/// A document parser directive (Firecrawl `parsers` entry). Accepts either the
/// bare string form (`"pdf"`) or the object form (`{ "type": "pdf",
/// "mode": "auto", "maxPages": 10 }`) on the wire; always serializes to the
/// object form. Matches Firecrawl v2's `parsers` shape exactly.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ParserSpec {
    /// Parser type. Only `"pdf"` is supported today.
    #[serde(rename = "type")]
    pub parser_type: String,
    /// Parsing strategy (Firecrawl: `auto` | `fast` | `ocr`). fastCRW has no
    /// OCR, so `ocr` degrades to text extraction with a warning, and `auto`
    /// (text-first + OCR fallback in Firecrawl) is text-only here. Accepted for
    /// wire-compatibility regardless. `None` ≈ `auto`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Optional cap on the number of pages to parse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_pages: Option<usize>,
}

impl ParserSpec {
    /// Convenience constructor for the common PDF directive.
    pub fn pdf() -> Self {
        Self {
            parser_type: "pdf".to_string(),
            mode: None,
            max_pages: None,
        }
    }
}

impl<'de> serde::Deserialize<'de> for ParserSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Str(String),
            Obj {
                #[serde(rename = "type")]
                parser_type: String,
                #[serde(default)]
                mode: Option<String>,
                #[serde(default, rename = "maxPages", alias = "max_pages")]
                max_pages: Option<usize>,
            },
        }
        Ok(match Raw::deserialize(deserializer)? {
            Raw::Str(parser_type) => ParserSpec {
                parser_type,
                mode: None,
                max_pages: None,
            },
            Raw::Obj {
                parser_type,
                mode,
                max_pages,
            } => ParserSpec {
                parser_type,
                mode,
                max_pages,
            },
        })
    }
}

fn default_formats() -> Vec<OutputFormat> {
    vec![OutputFormat::Markdown]
}

impl Default for ScrapeRequest {
    /// Matches the serde defaults exactly (`formats: ["markdown"]`,
    /// `only_main_content: true`, everything else empty/None). Hand-written
    /// rather than derived because `#[derive(Default)]` would give
    /// `formats: vec![]` / `only_main_content: false`, contradicting the wire
    /// defaults — the v2 adapters build `ScrapeRequest { .., ..Default::default() }`
    /// and rely on these matching.
    fn default() -> Self {
        Self {
            url: String::new(),
            formats: default_formats(),
            only_main_content: true,
            render_js: None,
            wait_for: None,
            include_tags: Vec::new(),
            exclude_tags: Vec::new(),
            json_schema: None,
            headers: HashMap::new(),
            css_selector: None,
            xpath: None,
            chunk_strategy: None,
            query: None,
            filter_mode: None,
            top_k: None,
            proxy: None,
            proxy_list: Vec::new(),
            proxy_rotation: None,
            country: None,
            stealth: None,
            actions: None,
            extract: None,
            llm_api_key: None,
            llm_provider: None,
            llm_model: None,
            base_url: None,
            summary_prompt: None,
            max_content_chars: None,
            renderer: None,
            deadline_ms: None,
            debug: None,
            change_tracking: None,
            goal: None,
            judge_enabled: None,
            parsers: None,
            screenshot_full_page: false,
        }
    }
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
    /// Number of pages, for paginated documents (PDF). `None` for web pages.
    /// Drives per-page billing on document scrapes / uploads. Serialized as
    /// `numPages` to match Firecrawl's metadata field name.
    #[serde(default, rename = "numPages", skip_serializing_if = "Option::is_none")]
    pub page_count: Option<usize>,
    /// Original filename for documents uploaded via `/v2/parse`. `None` for
    /// URL-sourced pages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_filename: Option<String>,
}

/// Token-usage and best-effort cost for one LLM call.
///
/// `estimated_cost_usd` is informational only — provider pricing drifts
/// and this value MUST NOT be used for customer billing.
///
/// `cache_hit_input_tokens` / `cache_miss_input_tokens` surface the
/// provider's prompt-cache breakdown (Anthropic `cache_read_input_tokens`,
/// OpenAI `prompt_tokens_details.cached_tokens`, DeepSeek
/// `prompt_cache_hit_tokens`). `None` means the provider did not report a
/// breakdown for this call. `truncated` flags requests whose markdown
/// input was clipped before the LLM call. `calls` aggregates the number
/// of underlying provider calls when usage is summed across multiple
/// invocations (default 1 for a single call).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_cost_usd: Option<f64>,
    pub model: String,
    pub provider: String,

    // ── Wave 2 additions (additive, backward-compatible via serde defaults) ──
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_hit_input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_miss_input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    #[serde(default = "one_u32", skip_serializing_if = "is_one_u32")]
    pub calls: u32,

    // ── Wave 4 (R1) additions: SaaS billing correlation across legs ──
    //
    // The SaaS-side managed pricing path needs to know exactly how many
    // summary calls executed AND whether the answer leg ran. The 5-branch
    // fail-closed dispatch keys off these counters:
    //   - executedSummaries > 0 OR answerExecuted ⇒ engine did work
    //   - inputTokens == 0 AND outputTokens == 0 ⇒ no upstream cost
    // Without the counters the SaaS cannot disambiguate "no work" from
    // "work but missing telemetry" and would refund or charge wrong.
    //
    // Always serialized (no skip_serializing_if) so the always-present
    // R1 invariant holds: when /v1/search returns llmUsage, both fields
    // are explicitly visible.
    #[serde(default)]
    pub executed_summaries: u32,
    #[serde(default)]
    pub answer_executed: bool,
}

fn one_u32() -> u32 {
    1
}
fn is_one_u32(n: &u32) -> bool {
    *n == 1
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
    /// Content fingerprint of the canonical markdown: hex SHA-256 of the
    /// normalized markdown (`crw_diff::snapshot::hash_markdown`). Stable across
    /// CRLF/whitespace noise, so clients can dedup/cache and evidence offsets
    /// (highlights, citations) can be tied to an exact source revision. `None`
    /// when no markdown was produced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
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
    /// LLM-generated summary; populated when `formats` includes `summary`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Token usage + best-effort cost for any LLM call this request triggered
    /// (summary, structured JSON, etc).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_usage: Option<LlmUsage>,
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
    /// Extraction debug trace; populated only when the request opts in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug_extraction: Option<DebugExtraction>,
    /// MIME content type of the fetched resource (from `FetchResult`).
    /// Surfaced so change-tracking can hash binary/non-text content (PDF,
    /// images) by bytes rather than attempting a markdown/json diff.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    /// Change-tracking result; populated only when `formats` includes
    /// `"changeTracking"`. Carries per-page status + diff (+ judgment when
    /// the orchestration layer ran the judge).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_tracking: Option<ChangeTrackingResult>,
    /// Page screenshot as a `data:image/png;base64,<...>` URL; populated only
    /// when `formats` includes `"screenshot"`. The `data:` prefix is wrapped
    /// once in `single.rs` (`FetchResult.screenshot` stays raw base64).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<String>,
}

/// Per-request extraction debug trace. One entry per extract() call
/// (multi-attempt JS escalation produces multiple attempts).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DebugExtraction {
    pub attempts: Vec<DebugAttempt>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugAttempt {
    pub renderer: String,
    pub extracted_via: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_features: Option<serde_json::Value>,
    pub candidates: Vec<DebugCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugCandidate {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_excerpt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cap_chars: Option<usize>,
    pub score: f64,
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
    /// 2-letter ISO 3166-1 alpha-2 country code (e.g. "us", "gb") applied to
    /// every page fetched in this crawl. See `ScrapeRequest::country`.
    #[serde(default)]
    pub country: Option<String>,
    /// Per-crawl proxy pool to rotate among (BYOP). Takes precedence over the
    /// server's configured pool. Empty = use server config. Rotation is applied
    /// per page (see `proxy_rotation`). Accepts the snake_case `proxy_list` alias.
    #[serde(default, alias = "proxy_list")]
    pub proxy_list: Vec<String>,
    /// Rotation strategy for `proxy_list` (`round_robin`, `random`,
    /// `sticky_per_host`). `None` = server default (`sticky_per_host`).
    #[serde(default, alias = "proxy_rotation")]
    pub proxy_rotation: Option<crate::proxy::ProxyRotation>,
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
            ("\"camoufox\"", RequestedRenderer::Camoufox),
        ] {
            let parsed: RequestedRenderer = serde_json::from_str(s).unwrap();
            assert_eq!(parsed, expected, "input {s} should parse to {expected:?}");
        }
    }

    #[test]
    fn requested_renderer_camoufox_round_trip() {
        let parsed: RequestedRenderer = serde_json::from_str("\"camoufox\"").unwrap();
        assert_eq!(parsed, RequestedRenderer::Camoufox);
        let json = serde_json::to_string(&RequestedRenderer::Camoufox).unwrap();
        assert_eq!(json, "\"camoufox\"");
        assert_eq!(
            resolve_pinned_renderer(Some(RequestedRenderer::Camoufox)),
            Some("camoufox")
        );
        assert_eq!(RendererKind::Camoufox.as_str(), "camoufox");
    }

    #[test]
    fn requested_renderer_chrome_proxy_round_trip() {
        let parsed: RequestedRenderer = serde_json::from_str("\"chrome_proxy\"").unwrap();
        assert_eq!(parsed, RequestedRenderer::ChromeProxy);
        let json = serde_json::to_string(&RequestedRenderer::ChromeProxy).unwrap();
        assert_eq!(json, "\"chrome_proxy\"");
        assert_eq!(
            resolve_pinned_renderer(Some(RequestedRenderer::ChromeProxy)),
            Some("chrome_proxy")
        );
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

    #[test]
    fn chrome_proxy_serializes_with_underscore() {
        let json = serde_json::to_string(&RendererKind::ChromeProxy).unwrap();
        assert_eq!(json, "\"chrome_proxy\"");
    }

    #[test]
    fn chrome_proxy_deserializes_from_underscore() {
        let k: RendererKind = serde_json::from_str("\"chrome_proxy\"").unwrap();
        assert_eq!(k, RendererKind::ChromeProxy);
    }

    #[test]
    fn chrome_proxy_as_str() {
        assert_eq!(RendererKind::ChromeProxy.as_str(), "chrome_proxy");
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
    /// When true (default), fall back to a short-budget BFS crawl after the
    /// sitemap phase to fill gaps. Set to false for sitemap-only mode — faster
    /// for sites with rich sitemaps, but may miss pages a sitemap omits.
    #[serde(default = "default_true")]
    pub crawl_fallback: bool,
    /// Custom timeout in seconds (default: 120).
    #[serde(default)]
    pub timeout: Option<u64>,
    /// Tier B — strip tracking params. `Some(_)` overrides TOML.
    #[serde(default)]
    pub strip_tracking_params: Option<bool>,
    /// Tier A — drop action URLs. `Some(_)` overrides TOML.
    #[serde(default)]
    pub drop_action_urls: Option<bool>,
    /// Firecrawl-compatible coarse alias. `Some(true)`: strip every
    /// non-preserved param. `Some(false)`: switch the whole filter off
    /// (raw URLs — the explicit escape hatch).
    #[serde(default)]
    pub ignore_query_parameters: Option<bool>,
    /// Additive on top of `DEFAULT_TRACKING_PARAMS`. Keys are normalized to
    /// canonical form (lowercase, `-` folded to `_`), so `add-to-cart` and
    /// `add_to_cart` are equivalent. Max 64 keys; over-cap → 422.
    #[serde(default)]
    pub extra_tracking_params: Option<Vec<String>>,
    /// Additive on top of `DEFAULT_ACTION_PARAMS`. Keys are normalized to
    /// canonical form (lowercase, `-` folded to `_`). Max 64 keys; over-cap → 422.
    #[serde(default)]
    pub extra_action_params: Option<Vec<String>>,
    /// Additive on top of `ALWAYS_PRESERVE` + TOML preserves. Keys are
    /// normalized to canonical form (lowercase, `-` folded to `_`).
    /// Max 64 keys; over-cap → 422.
    #[serde(default)]
    pub preserve_params: Option<Vec<String>>,
    /// Max URLs to discover. Firecrawl-compatible. Defaults to
    /// `DEFAULT_MAX_DISCOVERED_URLS`; the engine clamps to its hard ceiling.
    /// Raise it to dump large/nested sitemaps (e.g. songsterr's ~4.3M URLs).
    #[serde(default)]
    pub limit: Option<usize>,
}

/// POST /v1/map response data — the discovered links plus filter stats.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapData {
    pub links: Vec<String>,
    /// Number of URLs the /map filter dropped entirely (Tier A action-URL
    /// matches). `0` when the filter is disabled.
    #[serde(default)]
    pub dropped_action_count: usize,
    /// Number of URLs that had at least one query param stripped by Tier B.
    /// `0` when the filter is disabled.
    #[serde(default)]
    pub stripped_tracking_count: usize,
}

/// POST /v1/map response body.
/// Standard envelope: { success: true, data: { links: [...] } }
pub type MapResponse = ApiResponse<MapData>;

// ── Search types ──

/// Top-level "source" buckets exposed in the `/v1/search` API. Maps to
/// SearXNG's `categories` query parameter (web → general, news → news,
/// images → images).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchSource {
    Web,
    News,
    Images,
}

impl SearchSource {
    /// SearXNG category name for this source.
    pub fn searxng_category(self) -> &'static str {
        match self {
            SearchSource::Web => "general",
            SearchSource::News => "news",
            SearchSource::Images => "images",
        }
    }
}

/// User-facing category modifiers.
///
/// Three values carry curated, Firecrawl-compatible behavior:
/// - `Github` / `Research` switch to topical SearXNG *engines* (configurable
///   via `[search].github_engines` / `[search].research_engines`).
/// - `Pdf` appends `filetype:pdf` to the query.
///
/// Any other string is passed straight through to SearXNG's native
/// `categories` query parameter (e.g. `science`, `it`, `news`, `files`,
/// `images`), so SearXNG's own engine→category routing applies without any
/// crw code or config changes. This makes the surface a strict superset of
/// Firecrawl's `github`/`research`/`pdf` — existing callers are unaffected.
///
/// See <https://docs.searxng.org/user/configured_engines.html> for the
/// categories a given SearXNG instance exposes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchCategory {
    Github,
    Research,
    Pdf,
    /// Unknown value — forwarded verbatim to SearXNG's `categories` param.
    Other(String),
}

impl SearchCategory {
    /// Wire/string representation. The three curated variants round-trip to
    /// their lowercase names; `Other` returns the verbatim passthrough value.
    pub fn as_str(&self) -> &str {
        match self {
            SearchCategory::Github => "github",
            SearchCategory::Research => "research",
            SearchCategory::Pdf => "pdf",
            SearchCategory::Other(s) => s.as_str(),
        }
    }
}

impl From<String> for SearchCategory {
    fn from(s: String) -> Self {
        match s.as_str() {
            "github" => SearchCategory::Github,
            "research" => SearchCategory::Research,
            "pdf" => SearchCategory::Pdf,
            _ => SearchCategory::Other(s),
        }
    }
}

impl Serialize for SearchCategory {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SearchCategory {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(SearchCategory::from(String::deserialize(deserializer)?))
    }
}

/// Time-window filter, mirrors Google's `tbs=qdr:*` syntax used by Firecrawl.
/// SearXNG's `time_range` only supports day/week/month/year; `Hour` is mapped
/// to `Day` for parity with the SaaS implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchTimeFilter {
    #[serde(rename = "qdr:h")]
    Hour,
    #[serde(rename = "qdr:d")]
    Day,
    #[serde(rename = "qdr:w")]
    Week,
    #[serde(rename = "qdr:m")]
    Month,
    #[serde(rename = "qdr:y")]
    Year,
}

impl SearchTimeFilter {
    /// SearXNG `time_range` string. SearXNG has no hour granularity, so
    /// `Hour` is reported as `day` (lossy; matches SaaS behavior).
    pub fn searxng_time_range(self) -> &'static str {
        match self {
            SearchTimeFilter::Hour | SearchTimeFilter::Day => "day",
            SearchTimeFilter::Week => "week",
            SearchTimeFilter::Month => "month",
            SearchTimeFilter::Year => "year",
        }
    }
}

/// `scrapeOptions` sub-object — a narrow projection of `ScrapeRequest` that
/// we accept on every result from a search. Only the fields the SaaS exposes.
///
/// `formats` defaults to `["markdown"]` so Firecrawl-compatible callers that
/// pass `scrapeOptions: {}` (toggle enrichment without specifying formats)
/// get a sensible default instead of a deserialization error.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchScrapeOptions {
    #[serde(default = "default_formats")]
    pub formats: Vec<OutputFormat>,
    #[serde(default = "default_true")]
    pub only_main_content: bool,
    /// Residential-proxy exit country (ISO 3166-1 alpha-2) for the per-result page scrape.
    /// Populated by the SaaS layer from the caller's IP (geo-aware proxy). `None` = engine default.
    #[serde(default)]
    pub country: Option<String>,
}

/// POST /v1/search request body. Mirrors the zod schema in
/// `crw-saas/src/lib/search-schema.ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    pub query: String,
    /// Number of results per source (or total when `sources` is unset).
    /// Defaults to `[search].default_limit` when omitted; clamped to
    /// `[search].max_limit` server-side.
    #[serde(default)]
    pub limit: Option<u32>,
    /// SearXNG `language` parameter (e.g. `"en"`, `"de"`, `"auto"`).
    #[serde(default)]
    pub lang: Option<String>,
    /// Google-style time filter (`qdr:h|d|w|m|y`).
    #[serde(default)]
    pub tbs: Option<SearchTimeFilter>,
    /// When set, results are grouped under `web`/`news`/`images` keys.
    /// When unset, a flat array is returned.
    #[serde(default)]
    pub sources: Option<Vec<SearchSource>>,
    /// User-facing category modifiers. Max 5 entries (matches SaaS).
    #[serde(default)]
    pub categories: Option<Vec<SearchCategory>>,
    /// When set, every `web` result is enriched in-process via the scrape
    /// pipeline (parallel, bounded by `[crawler].max_concurrency`).
    #[serde(default)]
    pub scrape_options: Option<SearchScrapeOptions>,
    /// When true, every scraped result also gets an LLM summary attached to
    /// `SearchResult.summary`. Requires `scrape_options` to be set (so the
    /// markdown exists to summarize). LLM fan-out is bounded by
    /// `[extraction.llm].max_concurrency`.
    #[serde(default, alias = "summarize_results")]
    pub summarize_results: Option<bool>,
    /// When true, a single synthesized answer is generated from the top-N
    /// scraped results. Requires `scrape_options` to be set.
    #[serde(default)]
    pub answer: Option<bool>,
    /// Number of top results to include in answer synthesis (default 5,
    /// capped at 10).
    #[serde(default, alias = "answer_top_n")]
    pub answer_top_n: Option<u32>,
    /// Per-source character cap for the answer prompt (default 8192,
    /// hard-capped at 32768 server-side).
    #[serde(default, alias = "max_chars_per_source")]
    pub max_chars_per_source: Option<usize>,
    /// BYOK fields (mirror `ScrapeRequest`).
    #[serde(default, alias = "llm_api_key")]
    pub llm_api_key: Option<String>,
    #[serde(default, alias = "llm_provider")]
    pub llm_provider: Option<String>,
    #[serde(default, alias = "llm_model")]
    pub llm_model: Option<String>,
    #[serde(default, alias = "base_url")]
    pub base_url: Option<String>,
    /// Optional user-supplied instructions appended to the per-result
    /// summary system prompt. See `ScrapeRequest.summary_prompt`. Capped
    /// at 500 chars server-side.
    #[serde(default, alias = "summary_prompt")]
    pub summary_prompt: Option<String>,
    /// Optional user-supplied instructions appended to the answer-synthesis
    /// system prompt (e.g. "respond in Turkish", "be concise"). The
    /// "answer using ONLY the provided sources" rule and citation discipline
    /// stay intact. Capped at 500 chars server-side.
    #[serde(default, alias = "answer_prompt")]
    pub answer_prompt: Option<String>,
    /// Sampling temperature for the answer-synthesis LLM call. Omitted (None)
    /// keeps the provider default (prod behavior). The benchmark/eval harness
    /// sets `0` (with a fixed seed) to make answers deterministic, so a real
    /// accuracy lever is distinguishable from sampling noise.
    #[serde(default, alias = "answer_temperature")]
    pub answer_temperature: Option<f32>,
    /// Per-request override for `[search].query_expand_variants` — the number
    /// of diverse query rewrites fetched + unioned when query expansion is on.
    /// None uses the server config. The benchmark/eval harness sets this to A/B
    /// recall (e.g. 1 vs 3) at a fixed answer temperature.
    #[serde(default, alias = "query_expand_variants")]
    pub query_expand_variants: Option<usize>,
    /// Per-request override for `[search].multi_round` — the adaptive
    /// evidence-scout round that fires when the round-1 answer abstains. None
    /// uses the server config. The eval harness sets this to A/B the lever.
    #[serde(default, alias = "multi_round")]
    pub multi_round: Option<bool>,
    /// Per-request override for `[search].answer_list_format` — when the query
    /// has list intent ("best/top X in Y", "recommend …"), render the answer as
    /// a ranked list of named options instead of prose. None uses the server
    /// config; Some(false) forces prose, Some(true) forces the list path (still
    /// only fires on list-intent queries).
    #[serde(default, alias = "answer_list_format")]
    pub answer_list_format: Option<bool>,
    /// Maximum number of bytes of each per-result markdown sent to the LLM
    /// when `summarize_results` is enabled. Defaults to
    /// `[extraction.llm].max_html_bytes` (100 KB). Clamped to a 200 KB
    /// server-side ceiling. Independent from `max_chars_per_source`, which
    /// caps the answer-synthesis path, not the per-result summary path.
    #[serde(default, alias = "max_content_chars")]
    pub max_content_chars: Option<usize>,
}

/// A single search result (web or news). Mirrors `SearchResult` in
/// `crw-saas/src/lib/search-transform.ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub url: String,
    pub title: String,
    pub description: String,
    /// Alias of `description`. Always populated. Emitted so downstream LLM
    /// pipelines that ask for "snippet" (Firecrawl convention) don't need a
    /// rename step. `#[serde(default)]` keeps deserialization permissive for
    /// callers that don't supply it.
    #[serde(default)]
    pub snippet: String,
    pub position: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    // Populated when scrapeOptions is used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markdown: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_html: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<PageMetadata>,
    /// LLM-generated summary; populated when `summarizeResults: true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// A single image result. Mirrors `ImageResult` in
/// `crw-saas/src/lib/search-transform.ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageResult {
    pub url: String,
    pub title: String,
    pub description: String,
    pub image_url: String,
    pub position: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
}

/// Grouped result envelope when `sources` is set on the request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupedSearchData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web: Option<Vec<SearchResult>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub news: Option<Vec<SearchResult>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<ImageResult>>,
}

/// `data` payload of the `/v1/search` response. Either a flat list of
/// results or a grouped object — chosen by whether the request specified
/// `sources`. Untagged: serializes as either an array or an object with
/// `web`/`news`/`images` keys.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SearchData {
    Flat(Vec<SearchResult>),
    Grouped(GroupedSearchData),
}

/// A citation reference in an LLM-synthesized search answer. `position`
/// is clamped to `[0, sources.len())` server-side so fabricated indices
/// can't escape the input source list.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Citation {
    pub url: String,
    pub title: String,
    pub position: u32,
}

/// Wrapper data envelope for `/v1/search` responses. Carries the existing
/// `SearchData` (flat or grouped) alongside optional LLM-generated
/// `answer` + `citations`. Adding sibling fields directly to `SearchData`
/// is impossible because that enum is `#[serde(untagged)]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponseData {
    pub results: SearchData,
    /// LLM-synthesized answer over the top-N results; `None` unless
    /// `answer: true` was set on the request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    /// Source citations for the answer. Order is meaningful: citation #0
    /// is `sources[0]`. Capped at 20 entries.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub citations: Vec<Citation>,
    /// Token usage + best-effort cost from the answer synthesis call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_usage: Option<LlmUsage>,
    /// Soft-failure / partial-result notices (e.g. "answer call rate-limited;
    /// summaries returned without answer").
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}

/// POST /v1/search response body.
pub type SearchResponse = ApiResponse<SearchResponseData>;

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
    #[serde(rename = "chrome_proxy")]
    ChromeProxy,
    /// Opt-in Camoufox stealth tier (REST). `rename_all = "lowercase"` yields
    /// `"camoufox"`. Unconditional (not feature-gated) like every other kind —
    /// the variant is inert in lean builds since no camoufox renderer is ever
    /// constructed there.
    Camoufox,
}

impl RendererKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RendererKind::Http => "http",
            RendererKind::Lightpanda => "lightpanda",
            RendererKind::Chrome => "chrome",
            RendererKind::ChromeProxy => "chrome_proxy",
            RendererKind::Camoufox => "camoufox",
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
    /// Vendor-specific anti-bot block (Akamai, PerimeterX, DataDome, etc.).
    /// Vendor name is recorded via `crw_vendor_block_total{vendor}` metric
    /// and the renderer warning — not carried in the enum variant to keep
    /// the type `Copy`-friendly.
    VendorBlock,
    /// JS renderer returned a 4xx/5xx HTTP status (e.g. 403, 429) — same
    /// status set the HTTP tier escalates on. Caught in the JS tier so a
    /// "200 with bot HTML" or "403 with content" can't masquerade as success.
    StatusBlocked,
    /// The `crw_extract::antibot` classifier flagged a block the lighter
    /// `detector` heuristics missed (e.g. a "blocked by network security"
    /// WAF page served with HTTP 200). Drives escalation toward the
    /// residential `chrome_proxy` tier; counts toward host promotion.
    AntibotBlock,
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
                | FailoverErrorKind::AntibotBlock
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
            FailoverErrorKind::VendorBlock => "vendorBlock",
            FailoverErrorKind::StatusBlocked => "statusBlocked",
            FailoverErrorKind::AntibotBlock => "antibotBlock",
            FailoverErrorKind::NetworkError => "networkError",
            FailoverErrorKind::Other => "other",
        }
    }
}

/// Result of fetching + optionally rendering a page.
#[derive(Debug, Clone)]
pub struct FetchResult {
    pub url: String,
    /// Final URL after redirects, populated only when it differs from the
    /// requested `url`. None means no redirect or scheme/path was identical.
    pub final_url: Option<String>,
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
    /// Set by chrome renderer when the navigation budget elapsed before
    /// `loadEventFired` and we snapshotted the partial DOM. Mid-load HTML may
    /// still extract usefully (`single.rs` decides success on md length).
    pub truncated: bool,
    /// Set when `Deadline::remaining() == 0` was observed at result-build time.
    /// Stricter than `truncated` — caller's whole budget is spent.
    pub deadline_exceeded: bool,
    /// XHR/fetch responses captured during navigation. Empty unless the
    /// renderer ran with network capture enabled. Used by extraction as a
    /// fallback content source when DOM-based extraction is low quality.
    pub captured_responses: Vec<CapturedNetworkResponse>,
    /// Raw base64 PNG captured via CDP `Page.captureScreenshot` when the
    /// request asked for the `screenshot` format. `None` for the HTTP /
    /// camoufox / lightpanda paths (they never capture). The `data:` URL
    /// prefix is added in `single.rs`, not here.
    pub screenshot: Option<String>,
}

/// A single XHR/fetch response captured via CDP Network domain.
#[derive(Debug, Clone)]
pub struct CapturedNetworkResponse {
    pub url: String,
    pub request_id: String,
    pub status: u16,
    pub mime_type: Option<String>,
    pub body: Option<String>,
    pub body_size_bytes: usize,
}

// ===========================================================================
// Change tracking (monitor) types
//
// These types are the stateless primitives the SaaS / self-host monitor
// control plane builds on. `crw-diff` consumes `ChangeTrackingOptions` and
// produces a `ChangeTrackingResult`; the LLM judge (`crw-extract`) populates
// `ChangeJudgment`. Wire shapes mirror Firecrawl's `/monitor` check payloads.
// ===========================================================================

/// Change-tracking diff mode. Wire: `"gitDiff"` or `"json"`.
///
/// Deserialization also accepts `"git-diff"` for ergonomics; serialization
/// always emits the canonical `"gitDiff"` / `"json"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeTrackingMode {
    GitDiff,
    Json,
}

impl<'de> Deserialize<'de> for ChangeTrackingMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "gitDiff" | "git-diff" => Ok(ChangeTrackingMode::GitDiff),
            "json" => Ok(ChangeTrackingMode::Json),
            other => Err(serde::de::Error::custom(format!(
                "Unknown changeTracking mode '{other}'. Valid modes: gitDiff, json (alias: git-diff)."
            ))),
        }
    }
}

/// A snapshot of a scrape, used as the baseline to diff against. The caller
/// (SaaS / self-host monitor) persists this between checks and supplies the
/// prior one as `previous`; opencore is stateless and stores nothing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeTrackingSnapshot {
    /// Normalized markdown content (present for gitDiff / mixed mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub markdown: Option<String>,
    /// Extracted structured JSON (present for json / mixed mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json: Option<serde_json::Value>,
    /// Mode-aware content hash (markdown hash for gitDiff/mixed; tracked-field
    /// hash for json mode). The SaaS short-circuit keys off this.
    #[serde(default)]
    pub content_hash: String,
    /// Caller-stamped capture time; echoed back untouched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captured_at: Option<String>,
}

/// Change-tracking options. Sibling field on `ScrapeRequest` (activated by the
/// `"changeTracking"` format string) and the body of `POST /v1/change-tracking/diff`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeTrackingOptions {
    /// Diff surfaces to compute. `["gitDiff"]` = markdown unified diff + AST;
    /// `["json"]` = per-field diff; `["json","gitDiff"]` = mixed (both).
    #[serde(default)]
    pub modes: Vec<ChangeTrackingMode>,
    /// JSON schema describing the fields to track (json / mixed mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<serde_json::Value>,
    /// Natural-language extraction prompt (alternative to `schema`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// The previous snapshot to diff against. `None` => first observation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<ChangeTrackingSnapshot>,
    /// Opaque caller tag echoed back on the result (e.g. a target id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// MIME content type of the current page (binary/non-text → byte hash, no diff).
    #[serde(
        default,
        alias = "content_type",
        skip_serializing_if = "Option::is_none"
    )]
    pub content_type: Option<String>,
}

/// Per-page change status emitted by opencore. Set-level `new` / `removed`
/// are computed by the caller's reconciler, not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeStatus {
    Same,
    Changed,
}

/// Judge confidence level. Matches Firecrawl's `"low" | "medium" | "high"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeConfidence {
    Low,
    Medium,
    High,
}

/// A single meaningful change called out by the judge. Mirrors Firecrawl's
/// `meaningfulChanges[]` entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeaningfulChange {
    /// `"added" | "removed" | "changed"`.
    #[serde(rename = "type")]
    pub change_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    pub reason: String,
}

/// LLM meaningful-change judgment. Public wire shape is exactly
/// `{meaningful, confidence, reason, meaningfulChanges}` (Firecrawl parity);
/// `llm_usage` is internal-only and never serialized.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeJudgment {
    pub meaningful: bool,
    pub confidence: ChangeConfidence,
    pub reason: String,
    #[serde(default)]
    pub meaningful_changes: Vec<MeaningfulChange>,
    /// Token usage for the judge call. Internal-only — `skip` keeps it out of
    /// the public judgment wire shape; the orchestration layer reads it for
    /// billing/observability.
    #[serde(skip)]
    pub llm_usage: Option<LlmUsage>,
}

/// One change line within a diff chunk (parse-diff-compatible).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffChange {
    /// `"add" | "del" | "normal"`.
    #[serde(rename = "type")]
    pub change_type: String,
    pub content: String,
    /// New-file line number (add / normal).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ln: Option<usize>,
    /// Old-file line number (normal only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ln1: Option<usize>,
    /// New-file line number (normal only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ln2: Option<usize>,
}

/// A hunk within a diff file (parse-diff-compatible).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffChunk {
    /// The `@@ -a,b +c,d @@` header line.
    pub content: String,
    pub changes: Vec<DiffChange>,
    pub old_start: usize,
    pub old_lines: usize,
    pub new_start: usize,
    pub new_lines: usize,
}

/// A single file's diff (parse-diff-compatible). For a single-page change
/// track there is always exactly one synthetic file (`previous` → `current`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffFile {
    pub from: String,
    pub to: String,
    pub additions: usize,
    pub deletions: usize,
    pub chunks: Vec<DiffChunk>,
}

/// The git-diff AST (parse-diff style). Serialized into `diff.json` for
/// gitDiff-only mode; in mixed mode the per-field json diff takes `diff.json`
/// instead and this AST is not surfaced.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DiffAst {
    pub files: Vec<DiffFile>,
    pub additions: usize,
    pub deletions: usize,
    /// True when the AST was capped at `max_diff_changes` (full snapshot still
    /// retained, so the change is recoverable).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

/// The `diff` envelope: `{ text?, json? }`. `text` is the unified markdown
/// diff (gitDiff / mixed). `json` is mode-polymorphic — the parse-diff AST in
/// gitDiff-only mode, or the per-field path map (`{ "<path>": {previous,current} }`)
/// in json / mixed mode. Modeled as `Value` to carry either shape, exactly
/// matching Firecrawl's wire payload.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeDiff {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json: Option<serde_json::Value>,
}

/// Result of a change-tracking computation for one page. Surfaced on
/// `ScrapeData.change_tracking` and returned by `POST /v1/change-tracking/diff`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeTrackingResult {
    pub status: ChangeStatus,
    /// True when no `previous` was supplied — the caller maps this to `new`.
    #[serde(default)]
    pub first_observation: bool,
    /// Mode-aware hash of the current content (see `ChangeTrackingSnapshot`).
    pub content_hash: String,
    /// The current snapshot — persist this as the next check's `previous`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<ChangeTrackingSnapshot>,
    /// The diff surfaces; `None` when `status == Same` or for binary content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<ChangeDiff>,
    /// Meaningful-change judgment; populated by the orchestration layer only
    /// when the page changed, a goal is set, and judging is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judgment: Option<ChangeJudgment>,
    /// Echoed caller tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// True when the diff AST was truncated (mirrors `DiffAst.truncated`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}
