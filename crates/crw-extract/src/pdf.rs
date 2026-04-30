//! PDF content extraction via pdf-inspector (lopdf-based).
//!
//! Converts raw PDF bytes into markdown, reusing the same library that Firecrawl uses.

use crw_core::error::{CrwError, CrwResult};
use crw_core::types::{OutputFormat, PageMetadata, ScrapeData};

/// Hard cap on PDF byte length we'll attempt to parse synchronously. Above
/// this, the underlying lopdf-based parser can pin a worker thread for 60+
/// seconds (observed on meti.go.jp's 8 MB report PDF) which then surfaces as
/// a 90s client timeout for the caller. Falling fast with a clear warning is
/// strictly better than the timeout — we surface enough metadata that the
/// caller can route the URL to a streaming PDF handler if they need one.
const MAX_PDF_PARSE_BYTES: usize = 5 * 1024 * 1024;

/// Extract content from raw PDF bytes and produce a [`ScrapeData`] result.
pub fn extract_pdf(
    bytes: &[u8],
    source_url: &str,
    status_code: u16,
    elapsed_ms: u64,
    formats: &[OutputFormat],
) -> CrwResult<ScrapeData> {
    if bytes.len() > MAX_PDF_PARSE_BYTES {
        let metadata = PageMetadata {
            title: None,
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
        return Ok(ScrapeData {
            markdown: None,
            html: None,
            raw_html: None,
            plain_text: None,
            links: None,
            json: None,
            chunks: None,
            warning: Some(format!(
                "pdf_too_large: {} bytes (max {MAX_PDF_PARSE_BYTES})",
                bytes.len()
            )),
            warnings: Vec::new(),
            render_decision: None,
            credit_cost: 0,
            metadata,
        });
    }
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
        warnings: Vec::new(),
        render_decision: None,
        credit_cost: 0,
        metadata,
    })
}
