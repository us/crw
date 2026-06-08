//! Integration tests for the PDF → markdown adapter (`crw_extract::pdf`).
//! Gated on the `pdf` feature (default-on).

#![cfg(feature = "pdf")]

use crw_extract::pdf::{self, PdfError};

const SAMPLE: &[u8] = include_bytes!("fixtures/sample.pdf");

#[test]
fn text_pdf_extracts_markdown_and_plaintext() {
    let r = pdf::convert(SAMPLE, true, None, 0).expect("text PDF should convert");
    assert!(!r.is_scanned, "sample.pdf is text-based, not scanned");
    assert_eq!(r.page_count, 2, "fixture has two pages");
    assert!(
        r.markdown.contains("Hello fastCRW PDF parsing"),
        "markdown should contain the first-page heading; got: {}",
        r.markdown
    );
    assert!(
        r.markdown.contains("Second page content"),
        "markdown should include the second page"
    );
    assert!(
        r.plain_text.contains("Hello fastCRW PDF parsing"),
        "plaintext should carry the same text"
    );
}

#[test]
fn plaintext_skipped_when_not_requested() {
    let r = pdf::convert(SAMPLE, false, None, 0).expect("convert");
    assert!(!r.markdown.is_empty());
    assert!(
        r.plain_text.is_empty(),
        "plaintext pass is skipped when want_plaintext=false"
    );
}

#[test]
fn max_pages_caps_conversion() {
    let r = pdf::convert(SAMPLE, false, Some(1), 0).expect("convert with page cap");
    assert!(
        r.markdown.contains("Hello fastCRW PDF parsing"),
        "first page is present"
    );
    assert!(
        !r.markdown.contains("Second page content"),
        "second page should be excluded by max_pages=1"
    );
}

#[test]
fn corrupt_bytes_error_without_panic() {
    // A header that looks like a PDF but is garbage must not panic (catch_unwind)
    // and must surface a parse error.
    let res = pdf::convert(b"%PDF-1.4\nthis is not a real pdf body", false, None, 0);
    assert!(res.is_err(), "corrupt PDF should error");
}

#[test]
fn non_pdf_bytes_rejected() {
    let res = pdf::convert(b"<html><body>not a pdf</body></html>", false, None, 0);
    assert!(matches!(
        res,
        Err(PdfError::NotAPdf) | Err(PdfError::Corrupt(_))
    ));
}

#[test]
fn error_codes_are_stable() {
    assert_eq!(PdfError::Encrypted.code(), "pdf_encrypted");
    assert_eq!(PdfError::NotAPdf.code(), "pdf_not_a_pdf");
    assert_eq!(PdfError::Corrupt("x".into()).code(), "pdf_parse_failed");
    assert_eq!(PdfError::Disabled.code(), "pdf_disabled");
}
