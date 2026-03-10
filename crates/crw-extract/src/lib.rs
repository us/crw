//! HTML content extraction and format conversion for the CRW web scraper.
//!
//! Converts raw HTML into clean, structured output formats:
//!
//! - **Markdown** — via [`markdown::html_to_markdown`] (htmd)
//! - **Plain text** — via [`plaintext::html_to_plaintext`]
//! - **Cleaned HTML** — boilerplate removal with [`clean::clean_html`]
//! - **Readability** — main-content extraction with text-density scoring
//! - **CSS/XPath selector** — narrow content to a specific element
//! - **Chunking** — split content into sentence/topic/regex chunks
//! - **Filtering** — BM25 or cosine-similarity ranking of chunks
//! - **Structured JSON** — LLM-based extraction with JSON Schema validation

pub mod chunking;
pub mod clean;
pub mod filter;
pub mod markdown;
pub mod plaintext;
pub mod readability;
pub mod selector;
pub mod structured;

use crw_core::error::{CrwError, CrwResult};
use crw_core::types::{ChunkStrategy, FilterMode, OutputFormat, PageMetadata, ScrapeData};

/// Options for the high-level extraction pipeline.
pub struct ExtractOptions<'a> {
    pub raw_html: &'a str,
    pub source_url: &'a str,
    pub status_code: u16,
    pub rendered_with: Option<String>,
    pub elapsed_ms: u64,
    pub formats: &'a [OutputFormat],
    pub only_main_content: bool,
    pub include_tags: &'a [String],
    pub exclude_tags: &'a [String],
    /// CSS selector to narrow content before readability extraction.
    pub css_selector: Option<&'a str>,
    /// XPath expression to narrow content before readability extraction.
    pub xpath: Option<&'a str>,
    /// Strategy for chunking the extracted markdown.
    pub chunk_strategy: Option<&'a ChunkStrategy>,
    /// Query for chunk filtering (requires filter_mode).
    pub query: Option<&'a str>,
    /// Filtering algorithm for chunk ranking.
    pub filter_mode: Option<&'a FilterMode>,
    /// Number of top chunks to return (default: 5).
    pub top_k: Option<usize>,
}

/// High-level extraction: given raw HTML + options, produce ScrapeData.
pub fn extract(opts: ExtractOptions<'_>) -> CrwResult<ScrapeData> {
    let ExtractOptions {
        raw_html,
        source_url,
        status_code,
        rendered_with,
        elapsed_ms,
        formats,
        only_main_content,
        include_tags,
        exclude_tags,
        css_selector,
        xpath,
        chunk_strategy,
        query,
        filter_mode,
        top_k,
    } = opts;

    // Step 1: Extract metadata from raw HTML.
    let meta = readability::extract_metadata(raw_html);

    // Step 2: Clean HTML (remove boilerplate, nav, ads, etc.).
    let cleaned = clean::clean_html(raw_html, only_main_content, include_tags, exclude_tags)
        .unwrap_or_else(|_| raw_html.to_string());

    // Step 3: Apply CSS/XPath selector if provided (narrows to a specific element).
    let selected_html = apply_selector(&cleaned, css_selector, xpath)?;
    let after_selection = selected_html.as_deref().unwrap_or(&cleaned);

    // Step 4: If only_main_content, try to narrow further with readability scoring.
    let (content_html, cleaned_ref) = if only_main_content && selected_html.is_none() {
        let main = readability::extract_main_content(after_selection);
        (main, Some(cleaned))
    } else {
        (after_selection.to_string(), None)
    };

    // Step 5: Produce requested formats.
    let md = if formats.contains(&OutputFormat::Markdown) || formats.contains(&OutputFormat::Json) {
        let md = markdown::html_to_markdown(&content_html);
        // Trigger fallback if markdown is empty OR suspiciously short relative to HTML.
        // Skip fallback when a CSS/XPath selector was explicitly used — short output is intentional.
        let md_too_short =
            selected_html.is_none() && md.trim().len() < 100 && raw_html.len() > 5000;
        if md_too_short {
            let fallback_md = if only_main_content && selected_html.is_none() {
                // Fallback 1: cleaned HTML without main-content narrowing
                let from_cleaned = if let Some(ref cleaned) = cleaned_ref {
                    markdown::html_to_markdown(cleaned)
                } else {
                    String::new()
                };
                if from_cleaned.trim().is_empty() {
                    // Fallback 2: basic clean (no only_main_content stripping)
                    let basic_cleaned =
                        clean::clean_html(raw_html, false, include_tags, exclude_tags)
                            .unwrap_or_else(|_| raw_html.to_string());
                    markdown::html_to_markdown(&basic_cleaned)
                } else {
                    from_cleaned
                }
            } else {
                markdown::html_to_markdown(raw_html)
            };

            let fallback_too_short = fallback_md.trim().len() < 100 && raw_html.len() > 5000;
            if fallback_too_short {
                let text = plaintext::html_to_plaintext(&content_html);
                if text.trim().is_empty() {
                    let basic_cleaned =
                        clean::clean_html(raw_html, false, include_tags, exclude_tags)
                            .unwrap_or_else(|_| raw_html.to_string());
                    Some(plaintext::html_to_plaintext(&basic_cleaned))
                } else {
                    Some(text)
                }
            } else {
                Some(fallback_md)
            }
        } else {
            Some(md)
        }
    } else {
        None
    };

    let plain = if formats.contains(&OutputFormat::PlainText) {
        Some(plaintext::html_to_plaintext(&content_html))
    } else {
        None
    };

    let raw = if formats.contains(&OutputFormat::RawHtml) {
        Some(raw_html.to_string())
    } else {
        None
    };

    let html = if formats.contains(&OutputFormat::Html) {
        Some(content_html)
    } else {
        None
    };

    let links = if formats.contains(&OutputFormat::Links) {
        Some(readability::extract_links(raw_html, source_url))
    } else {
        None
    };

    // JSON extraction is handled asynchronously in scrape_url after extract() returns.
    let json = None;

    // Step 6: Chunk the markdown if a strategy is provided.
    let chunks = if let Some(strategy) = chunk_strategy
        && let Some(ref markdown_text) = md
        && !markdown_text.trim().is_empty()
    {
        let raw_chunks = chunking::chunk_text(markdown_text, strategy);

        // Step 7: Filter chunks by relevance if query + filter_mode are set.
        let filtered = if let (Some(q), Some(mode)) = (query, filter_mode)
            && !q.trim().is_empty()
            && !raw_chunks.is_empty()
        {
            filter::filter_chunks(&raw_chunks, q, mode, top_k.unwrap_or(5))
        } else {
            raw_chunks
        };

        if filtered.is_empty() {
            None
        } else {
            Some(filtered)
        }
    } else {
        None
    };

    Ok(ScrapeData {
        markdown: md,
        html,
        raw_html: raw,
        plain_text: plain,
        links,
        json,
        chunks,
        warning: None,
        metadata: PageMetadata {
            title: meta.title,
            description: meta.description,
            og_title: meta.og_title,
            og_description: meta.og_description,
            og_image: meta.og_image,
            canonical_url: meta.canonical_url,
            source_url: source_url.to_string(),
            language: meta.language,
            status_code,
            rendered_with,
            elapsed_ms,
        },
    })
}

/// Apply CSS selector or XPath to narrow HTML content.
/// Returns None if no selector is set or no match is found.
fn apply_selector(html: &str, css: Option<&str>, xpath: Option<&str>) -> CrwResult<Option<String>> {
    if let Some(sel) = css {
        let result = selector::extract_by_css(html, sel).map_err(CrwError::ExtractionError)?;
        if result.is_some() {
            return Ok(result);
        }
    }
    if let Some(xp) = xpath
        && let Some(texts) =
            selector::extract_by_xpath(html, xp).map_err(CrwError::ExtractionError)?
    {
        let wrapped = texts
            .into_iter()
            .map(|text| {
                let escaped = text
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                format!("<div>{escaped}</div>")
            })
            .collect::<Vec<_>>()
            .join("\n");
        return Ok(Some(wrapped));
    }
    Ok(None)
}
