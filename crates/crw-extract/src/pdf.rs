//! PDF → markdown adapter over the pure-Rust [`pdf_inspector`] crate.
//!
//! This is the single quarantine point for the `pdf-inspector` v0.1.0 API.
//! Everything the rest of the workspace needs flows through [`convert`] and
//! the [`PdfExtract`] / [`PdfError`] types, so an upstream API change touches
//! only this file.
//!
//! Behaviour:
//! - Classifies the document first (cheap, ~10–50ms) to flag scanned /
//!   image-only PDFs — `pdf-inspector` has NO OCR, so those yield empty or
//!   partial text plus a warning rather than an error.
//! - Runs the full detect→extract→markdown pipeline ([`process_pdf_mem`]).
//! - Backfills the document title from the PDF `/Info` dictionary.
//! - Wraps the (lopdf-backed) parse in [`catch_unwind`] — malformed PDFs can
//!   panic inside lopdf, and a panic must not take down a worker thread.
//!
//! When the `pdf` cargo feature is disabled the whole pipeline compiles to a
//! stub returning [`PdfError::Disabled`], so call sites build unconditionally.

use std::fmt;

use serde::{Deserialize, Serialize};

/// `true` when this crate was compiled with the `pdf` feature (PDF conversion
/// available). Surfaced via `/v2/capabilities` so SaaS frontends can gate the
/// upload UI on real support rather than assuming it.
pub const PDF_SUPPORTED: bool = cfg!(feature = "pdf");

/// Structured result of a PDF → markdown conversion. Serializable so it can
/// cross the sandbox subprocess boundary (worker → parent over a pipe).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PdfExtract {
    /// Concatenated markdown for the whole document.
    pub markdown: String,
    /// Plain-text rendition (markdown formatting stripped).
    pub plain_text: String,
    /// Number of pages in the document.
    pub page_count: usize,
    /// Title from the PDF metadata, if present.
    pub title: Option<String>,
    /// `true` when the document is scanned / image-only (no usable text layer).
    pub is_scanned: bool,
    /// Human-readable warnings (scanned, encoding issues, truncation, …).
    pub warnings: Vec<String>,
}

/// Failure modes for PDF conversion. Insulates the rest of the codebase from
/// the upstream `pdf_inspector::PdfError` so warning/HTTP-status mapping has a
/// stable surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PdfError {
    /// Password-protected / encrypted PDF (not supported — no key handling).
    Encrypted,
    /// Bytes are not a PDF (bad magic / header).
    NotAPdf,
    /// Structurally broken or unparseable PDF (also covers caught panics).
    Corrupt(String),
    /// PDF support was compiled out (`--no-default-features`).
    Disabled,
    /// Parse exceeded the configured wall-clock budget.
    Timeout,
    /// A FlateDecode stream decompresses beyond the configured byte cap
    /// (decompression-bomb guard) — rejected before the full payload is
    /// allocated.
    TooLarge,
}

impl PdfError {
    /// Stable, machine-friendly error code for API responses / warnings.
    pub fn code(&self) -> &'static str {
        match self {
            PdfError::Encrypted => "pdf_encrypted",
            PdfError::NotAPdf => "pdf_not_a_pdf",
            PdfError::Corrupt(_) => "pdf_parse_failed",
            PdfError::Disabled => "pdf_disabled",
            PdfError::Timeout => "pdf_timeout",
            PdfError::TooLarge => "pdf_too_large",
        }
    }

    /// Reconstruct an error from its [`code`](Self::code) — used to carry the
    /// failure back across the sandbox subprocess boundary.
    pub fn from_code(code: &str) -> PdfError {
        match code {
            "pdf_encrypted" => PdfError::Encrypted,
            "pdf_not_a_pdf" => PdfError::NotAPdf,
            "pdf_disabled" => PdfError::Disabled,
            "pdf_timeout" => PdfError::Timeout,
            "pdf_too_large" => PdfError::TooLarge,
            _ => PdfError::Corrupt("sandbox worker reported failure".to_string()),
        }
    }
}

impl fmt::Display for PdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PdfError::Encrypted => {
                write!(
                    f,
                    "pdf_encrypted: password-protected PDFs are not supported"
                )
            }
            PdfError::NotAPdf => write!(f, "pdf_not_a_pdf: bytes are not a valid PDF"),
            PdfError::Corrupt(detail) => write!(f, "pdf_parse_failed: {detail}"),
            PdfError::Disabled => write!(f, "pdf_disabled: PDF support not compiled in"),
            PdfError::Timeout => write!(f, "pdf_timeout: parse exceeded the time budget"),
            PdfError::TooLarge => write!(
                f,
                "pdf_too_large: document decompresses beyond the allowed size \
                 (possible decompression bomb)"
            ),
        }
    }
}

impl std::error::Error for PdfError {}

/// Convert raw PDF bytes into markdown (and, when `want_plaintext`, a
/// plain-text rendition).
///
/// This is pure-CPU and synchronous (lopdf + rayon under the hood) — callers
/// that run inside an async runtime MUST wrap it in `spawn_blocking`.
#[cfg(feature = "pdf")]
pub fn convert(
    bytes: &[u8],
    want_plaintext: bool,
    max_pages: Option<usize>,
    max_decompressed_bytes: usize,
) -> Result<PdfExtract, PdfError> {
    use pdf_inspector::{PdfError as UpstreamError, PdfOptions, PdfType};

    // Decompression-bomb guard FIRST: reject a file whose FlateDecode streams
    // inflate beyond the cap before pdf-inspector allocates the full payload.
    // Runs in bounded memory (a fixed read buffer), so a 5 MB → 5 GB bomb is
    // refused having allocated only kilobytes. `0` disables the guard.
    if max_decompressed_bytes > 0 {
        check_decompression_bomb(bytes, max_decompressed_bytes)?;
    }

    // Map upstream errors → our stable surface.
    fn map_err(e: UpstreamError) -> PdfError {
        match e {
            UpstreamError::Encrypted => PdfError::Encrypted,
            UpstreamError::NotAPdf(_) => PdfError::NotAPdf,
            UpstreamError::InvalidStructure => {
                PdfError::Corrupt("invalid PDF structure".to_string())
            }
            UpstreamError::Parse(msg) => PdfError::Corrupt(msg),
            UpstreamError::Io(e) => PdfError::Corrupt(format!("io error: {e}")),
        }
    }

    // lopdf can panic on adversarial input; isolate it so a bad PDF can't
    // unwind across the FFI/worker boundary. AssertUnwindSafe is sound here:
    // we only read `bytes` and return owned data — no shared mutable state
    // is left in an inconsistent state by a panic.
    let run = || -> Result<PdfExtract, PdfError> {
        // 1. Cheap classification for routing / scanned detection.
        let classification = pdf_inspector::classify_pdf_mem(bytes).map_err(map_err)?;
        let is_scanned = matches!(
            classification.pdf_type,
            PdfType::Scanned | PdfType::ImageBased
        );

        let mut warnings = Vec::new();
        if is_scanned {
            warnings.push(
                "pdf_scanned: document has no embedded text layer; OCR is not supported, \
                 extracted text may be empty or partial"
                    .to_string(),
            );
        }

        // 2. Full detect → extract → markdown pipeline (optionally page-capped).
        let result = match max_pages {
            Some(n) if n > 0 => {
                let opts = PdfOptions::new().pages(1..=(n as u32));
                pdf_inspector::process_pdf_mem_with_options(bytes, opts).map_err(map_err)?
            }
            _ => pdf_inspector::process_pdf_mem(bytes).map_err(map_err)?,
        };

        if result.has_encoding_issues {
            warnings.push(
                "pdf_encoding_issues: broken font encodings detected; some text may be garbled"
                    .to_string(),
            );
        }
        if !result.pages_needing_ocr.is_empty() && !is_scanned {
            warnings.push(format!(
                "pdf_partial_text: {} page(s) need OCR and were not fully extracted",
                result.pages_needing_ocr.len()
            ));
        }

        let markdown = result.markdown.unwrap_or_default();
        let plain_text = if want_plaintext {
            markdown_to_plain_text(&markdown)
        } else {
            String::new()
        };

        Ok(PdfExtract {
            markdown,
            plain_text,
            page_count: result.page_count as usize,
            title: result.title.filter(|t| !t.trim().is_empty()),
            is_scanned,
            warnings,
        })
    };

    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)) {
        Ok(res) => res,
        Err(_) => Err(PdfError::Corrupt("panic while parsing PDF".to_string())),
    }
}

/// Decompression-bomb guard. Structure-parses the PDF (cheap — does not eagerly
/// inflate page content) and bounded-inflates each FlateDecode stream, aborting
/// the moment the running decompressed total would exceed `cap`. Peak memory is
/// the 16 KiB read buffer, so a malicious file never gets to allocate its
/// multi-GB payload. Non-Flate streams are skipped (bounded by file size).
///
/// Conservative: if the structure can't be parsed or a stream isn't valid zlib,
/// we don't treat that as a bomb — the main parser will surface the real error.
#[cfg(feature = "pdf")]
fn check_decompression_bomb(bytes: &[u8], cap: usize) -> Result<(), PdfError> {
    use std::io::Read;

    use lopdf::{Document, Object};

    // If structure parse fails, skip the guard (not a bomb signal); the main
    // `process_pdf_mem` will produce the proper corrupt/encrypted error.
    let Ok(doc) = Document::load_mem(bytes) else {
        return Ok(());
    };

    let mut budget = cap;
    for obj in doc.objects.values() {
        let Object::Stream(stream) = obj else {
            continue;
        };
        let is_flate = match stream.dict.get(b"Filter") {
            Ok(Object::Name(n)) => n.as_slice() == b"FlateDecode",
            Ok(Object::Array(arr)) => arr
                .iter()
                .any(|f| matches!(f, Object::Name(n) if n.as_slice() == b"FlateDecode")),
            _ => false,
        };
        if !is_flate {
            continue;
        }

        let mut dec = flate2::read::ZlibDecoder::new(stream.content.as_slice());
        let mut buf = [0u8; 16 * 1024];
        loop {
            match dec.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if n > budget {
                        return Err(PdfError::TooLarge);
                    }
                    budget -= n;
                }
                // Not zlib / truncated — let the main parser decide; not a bomb.
                Err(_) => break,
            }
        }
    }
    Ok(())
}

/// Stub used when the `pdf` feature is disabled — keeps call sites compiling.
#[cfg(not(feature = "pdf"))]
pub fn convert(
    _bytes: &[u8],
    _want_plaintext: bool,
    _max_pages: Option<usize>,
    _max_decompressed_bytes: usize,
) -> Result<PdfExtract, PdfError> {
    Err(PdfError::Disabled)
}

/// Best-effort markdown → plain-text: strips the lightweight markdown markers
/// `pdf-inspector` emits (ATX headings, bold/italic, list bullets, link
/// syntax) so the `plainText` format is genuinely plain. Intentionally simple
/// — PDF markdown is mostly prose, not a full CommonMark document.
#[cfg(feature = "pdf")]
fn markdown_to_plain_text(md: &str) -> String {
    use once_cell::sync::Lazy;
    use regex::Regex;

    static LINK: Lazy<Regex> = Lazy::new(|| Regex::new(r"\[([^\]]*)\]\([^)]*\)").unwrap());
    static EMPH: Lazy<Regex> = Lazy::new(|| Regex::new(r"(\*\*|\*|__|_|`)").unwrap());

    let mut out = String::with_capacity(md.len());
    for line in md.lines() {
        let mut l = line.trim_end();
        // Drop leading ATX heading markers ("## ").
        l = l.trim_start_matches('#').trim_start();
        // Drop common list bullets.
        if let Some(rest) = l
            .strip_prefix("- ")
            .or_else(|| l.strip_prefix("* "))
            .or_else(|| l.strip_prefix("+ "))
        {
            l = rest;
        }
        let l = LINK.replace_all(l, "$1");
        let l = EMPH.replace_all(&l, "");
        out.push_str(l.trim_end());
        out.push('\n');
    }
    out.trim().to_string()
}

#[cfg(all(test, feature = "pdf"))]
mod tests {
    use super::*;

    #[test]
    fn markdown_strip_produces_plain_text() {
        let md = "# Title\n\nSome **bold** and *italic* and `code`.\n- item one\n[link](http://x)";
        let txt = markdown_to_plain_text(md);
        assert!(txt.contains("Title"));
        assert!(txt.contains("Some bold and italic and code."));
        assert!(txt.contains("item one"));
        assert!(txt.contains("link"));
        assert!(!txt.contains('*'));
        assert!(!txt.contains('#'));
        assert!(!txt.contains('['));
    }

    #[test]
    fn corrupt_bytes_do_not_panic() {
        let res = convert(b"%PDF-1.4 not really a pdf", false, None, 0);
        assert!(res.is_err(), "garbage should error, not panic");
    }

    #[test]
    fn non_pdf_bytes_error() {
        let res = convert(b"<html>hi</html>", false, None, 0);
        assert!(matches!(
            res,
            Err(PdfError::NotAPdf) | Err(PdfError::Corrupt(_))
        ));
    }

    #[test]
    fn decompression_bomb_rejected_before_alloc() {
        // Build a tiny PDF whose single content stream inflates to ~64 MB of
        // zeros from a few KB compressed. With a 1 MB cap it must be refused
        // with TooLarge (and the guard never allocates the 64 MB).
        use std::io::Write;
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        let chunk = vec![0u8; 1024 * 1024];
        for _ in 0..64 {
            enc.write_all(&chunk).unwrap();
        }
        let comp = enc.finish().unwrap();

        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.5\n");
        let mut offs = Vec::new();
        let objs: Vec<Vec<u8>> = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>".to_vec(),
            {
                let mut s = format!(
                    "<< /Length {} /Filter /FlateDecode >>\nstream\n",
                    comp.len()
                )
                .into_bytes();
                s.extend_from_slice(&comp);
                s.extend_from_slice(b"\nendstream");
                s
            },
        ];
        for (i, body) in objs.iter().enumerate() {
            offs.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            pdf.extend_from_slice(body);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref = pdf.len();
        pdf.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes(),
        );
        for o in &offs {
            pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
                objs.len() + 1,
                xref
            )
            .as_bytes(),
        );

        let res = convert(&pdf, false, None, 1024 * 1024);
        assert!(
            matches!(res, Err(PdfError::TooLarge)),
            "bomb should be rejected with TooLarge, got {res:?}"
        );

        // With the guard disabled (cap 0) the same file parses without the guard.
        let res2 = convert(&pdf, false, None, 0);
        assert!(res2.is_ok() || matches!(res2, Err(PdfError::Corrupt(_))));
    }
}
