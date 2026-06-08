//! Tests for the shared PDF conversion core (`crw_crawl::pdf`), which backs the
//! URL-scrape, crawl, upload, CLI, and MCP surfaces.

use crw_core::types::{OutputFormat, ParserSpec, ScrapeRequest};
use crw_crawl::pdf::{PdfSource, convert_pdf_bytes, convert_pdf_bytes_strict, pdf_parse_requested};

const SAMPLE_PDF: &[u8] = include_bytes!("fixtures/sample.pdf");

fn source() -> PdfSource {
    PdfSource {
        source_url: "https://example.com/doc.pdf".to_string(),
        status_code: 200,
        elapsed_ms: 5,
        source_filename: None,
    }
}

#[tokio::test]
async fn convert_builds_scrape_data_with_markdown_and_metadata() {
    let req = ScrapeRequest {
        formats: vec![OutputFormat::Markdown, OutputFormat::PlainText],
        ..Default::default()
    };
    let data = convert_pdf_bytes(SAMPLE_PDF.to_vec(), &req, source())
        .await
        .expect("conversion ok");

    assert!(
        data.markdown
            .as_deref()
            .unwrap_or("")
            .contains("Hello fastCRW PDF parsing"),
        "markdown populated"
    );
    assert!(data.plain_text.is_some(), "plaintext requested");
    assert!(data.html.is_none(), "PDFs have no HTML");
    assert_eq!(data.content_type.as_deref(), Some("application/pdf"));
    assert_eq!(data.metadata.rendered_with.as_deref(), Some("pdf"));
    assert_eq!(data.metadata.page_count, Some(2));
    // Per-page credit: 2 pages → 2 credits.
    assert_eq!(data.credit_cost, 2);
}

#[tokio::test]
async fn links_format_yields_empty_list_with_warning() {
    let req = ScrapeRequest {
        formats: vec![OutputFormat::Markdown, OutputFormat::Links],
        ..Default::default()
    };
    let data = convert_pdf_bytes(SAMPLE_PDF.to_vec(), &req, source())
        .await
        .unwrap();
    assert_eq!(data.links, Some(vec![]));
    assert!(
        data.warnings
            .iter()
            .any(|w| w.contains("pdf_links_unavailable")),
        "should warn that links aren't extracted from PDFs"
    );
}

#[tokio::test]
async fn corrupt_pdf_soft_fails_on_url_path() {
    let req = ScrapeRequest::default();
    let data = convert_pdf_bytes(b"%PDF-1.4 broken".to_vec(), &req, source())
        .await
        .expect("URL path returns Ok with a warning, never Err");
    assert!(data.markdown.as_deref().unwrap_or("").is_empty());
    assert!(data.warning.is_some(), "soft-fail surfaces a warning");
}

#[tokio::test]
async fn corrupt_pdf_hard_fails_on_upload_path() {
    let req = ScrapeRequest::default();
    let res = convert_pdf_bytes_strict(b"%PDF-1.4 broken".to_vec(), &req, source()).await;
    assert!(res.is_err(), "strict path errors for the upload endpoint");
}

#[test]
fn pdf_parse_requested_semantics() {
    // Field omitted → auto-parse (Firecrawl default).
    let mut req = ScrapeRequest::default();
    assert!(pdf_parse_requested(&req));

    // Explicit empty list → disabled.
    req.parsers = Some(vec![]);
    assert!(!pdf_parse_requested(&req));

    // List containing pdf → enabled.
    req.parsers = Some(vec![ParserSpec::pdf()]);
    assert!(pdf_parse_requested(&req));

    // List without pdf → not requested.
    req.parsers = Some(vec![ParserSpec {
        parser_type: "docx".into(),
        mode: None,
        max_pages: None,
    }]);
    assert!(!pdf_parse_requested(&req));
}

#[tokio::test]
async fn max_pages_parser_caps_output() {
    let req = ScrapeRequest {
        formats: vec![OutputFormat::Markdown],
        parsers: Some(vec![ParserSpec {
            parser_type: "pdf".into(),
            mode: None,
            max_pages: Some(1),
        }]),
        ..Default::default()
    };
    let data = convert_pdf_bytes(SAMPLE_PDF.to_vec(), &req, source())
        .await
        .unwrap();
    let md = data.markdown.unwrap_or_default();
    assert!(md.contains("Hello fastCRW PDF parsing"));
    assert!(
        !md.contains("Second page content"),
        "max_pages=1 caps output"
    );
}
