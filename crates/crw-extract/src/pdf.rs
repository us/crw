//! PDF content extraction via pdf-inspector (lopdf-based).
//!
//! Converts raw PDF bytes into markdown, reusing the same library that Firecrawl uses.

use crw_core::error::{CrwError, CrwResult};
use crw_core::types::{OutputFormat, PageMetadata, ScrapeData};

/// Extract content from raw PDF bytes and produce a [`ScrapeData`] result.
pub fn extract_pdf(
    bytes: &[u8],
    source_url: &str,
    status_code: u16,
    elapsed_ms: u64,
    formats: &[OutputFormat],
) -> CrwResult<ScrapeData> {
    let result = crw_pdf::process_pdf_mem(bytes)
        .map_err(|e| CrwError::ExtractionError(format!("PDF extraction failed: {e}")))?;

    let markdown = result.markdown.unwrap_or_default();

    let metadata = PageMetadata {
        title: result.title,
        description: None,
        og_title: None,
        og_description: None,
        og_image: None,
        canonical_url: None,
        source_url: source_url.to_string(),
        language: None,
        status_code,
        rendered_with: Some("pdf".to_string()),
        elapsed_ms,
    };

    let warning = match result.pdf_type {
        crw_pdf::PdfType::Scanned | crw_pdf::PdfType::ImageBased => {
            Some("PDF appears to be scanned/image-based — text extraction may be incomplete".into())
        }
        _ => None,
    };

    Ok(ScrapeData {
        markdown: if formats.contains(&OutputFormat::Markdown)
            || formats.contains(&OutputFormat::Json)
        {
            Some(markdown.clone())
        } else {
            None
        },
        html: None,
        raw_html: None,
        plain_text: if formats.contains(&OutputFormat::PlainText) {
            Some(markdown.clone())
        } else {
            None
        },
        links: None,
        json: None,
        chunks: None,
        warning,
        metadata,
    })
}
