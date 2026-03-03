pub mod clean;
pub mod markdown;
pub mod plaintext;
pub mod readability;
pub mod structured;

use crw_core::types::{OutputFormat, PageMetadata, ScrapeData};

/// High-level extraction: given raw HTML + options, produce ScrapeData.
#[allow(clippy::too_many_arguments)]
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
    let (content_html, cleaned_ref) = if only_main_content {
        let main = readability::extract_main_content(&cleaned);
        (main, Some(cleaned))
    } else {
        (cleaned, None)
    };

    // Step 4: Produce requested formats.
    let md = if formats.contains(&OutputFormat::Markdown) || formats.contains(&OutputFormat::Json) {
        let md = markdown::html_to_markdown(&content_html);
        // Trigger fallback if markdown is empty OR suspiciously short relative to HTML.
        // html2md can silently discard content for certain HTML structures.
        let md_too_short = md.trim().len() < 100 && raw_html.len() > 5000;
        if md_too_short {
            // html2md can fail on certain HTML structures (Framer, complex SPAs, etc.)
            // Fallback chain: try progressively less aggressive extraction.
            let fallback_md = if only_main_content {
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
                // Already not using only_main_content, try raw html directly
                markdown::html_to_markdown(raw_html)
            };

            let fallback_too_short = fallback_md.trim().len() < 100 && raw_html.len() > 5000;
            if fallback_too_short {
                // Last resort: extract plain text as markdown
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
