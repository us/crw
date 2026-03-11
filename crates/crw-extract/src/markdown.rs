/// Convert HTML to Markdown using htmd (turndown.js-inspired converter).
pub fn html_to_markdown(html: &str) -> String {
    let md = htmd::convert(html).unwrap_or_default();
    strip_anchor_artifacts(&md)
}

/// Remove pilcrow signs (¶) and other anchor-link artifacts that
/// HTML-to-Markdown converters carry over from header anchor links.
fn strip_anchor_artifacts(md: &str) -> String {
    md.replace('\u{00b6}', "") // pilcrow ¶
        .replace(" \u{00a7}", "") // section sign § (preceded by space)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_basic_html() {
        let html = "<h1>Title</h1><p>Paragraph with <strong>bold</strong> text.</p>";
        let md = html_to_markdown(html);
        assert!(md.contains("# Title"));
        assert!(md.contains("**bold**"));
    }

    #[test]
    fn strips_pilcrow_from_headers() {
        let html = r#"<h2>Section <a href="#section">¶</a></h2>"#;
        let md = html_to_markdown(html);
        assert!(!md.contains('\u{00b6}'));
        assert!(md.contains("Section"));
    }

    #[test]
    fn converts_links() {
        let html = r#"<p><a href="https://example.com">Link</a></p>"#;
        let md = html_to_markdown(html);
        assert!(md.contains("[Link](https://example.com)"));
    }
}
