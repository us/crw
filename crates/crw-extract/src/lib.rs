pub mod clean;
pub mod markdown;
pub mod plaintext;
pub mod readability;
pub mod structured;

use crw_core::types::{OutputFormat, PageMetadata, ScrapeData};

/// High-level extraction: given raw HTML + options, produce ScrapeData.
pub fn extract(
    raw_html: &str,
    source_url: &str,
    status_code: u16,
    rendered_with: Option<String>,
    elapsed_ms: u64,
    formats: &[OutputFormat],
    only_main_content: bool,
    include_tags: &[String],
    exclude_tags: &[String],
) -> ScrapeData {
    // Step 1: Extract metadata from raw HTML.
    let meta = readability::extract_metadata(raw_html);

    // Step 2: Clean HTML.
    let cleaned = clean::clean_html(raw_html, only_main_content, include_tags, exclude_tags)
        .unwrap_or_else(|_| raw_html.to_string());

    // Step 3: If only_main_content, try to narrow down to main element.
    let content_html = if only_main_content {
        readability::extract_main_content(&cleaned)
    } else {
        cleaned
    };

    // Step 4: Produce requested formats.
    let md = if formats.contains(&OutputFormat::Markdown) || formats.contains(&OutputFormat::Json) {
        Some(markdown::html_to_markdown(&content_html))
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

    // Move content_html into html output (avoids clone).
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

    ScrapeData {
        markdown: md,
        html,
        raw_html: raw,
        plain_text: plain,
        links,
        json,
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
    }
}
