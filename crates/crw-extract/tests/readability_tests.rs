use crw_extract::readability::{extract_links, extract_main_content, extract_metadata};

// ── Main Content Extraction ──

#[test]
fn extract_main_content_no_article_falls_to_body() {
    let html = "<html><body><p>Body content only</p></body></html>";
    let content = extract_main_content(html);
    assert!(content.contains("Body content only"));
}

#[test]
fn extract_main_content_multiple_selectors_priority() {
    // article should take priority over main
    let html = "<html><body><main><p>Main content</p></main><article><p>Article content</p></article></body></html>";
    let content = extract_main_content(html);
    assert!(
        content.contains("Article content"),
        "article should have priority over main. Got: {content}"
    );
}

#[test]
fn extract_main_content_uses_role_main() {
    let html = r#"<html><body><div role="main"><p>Role main content</p></div></body></html>"#;
    let content = extract_main_content(html);
    assert!(content.contains("Role main content"));
}

#[test]
fn extract_main_content_no_body() {
    let html = "<p>Just a paragraph</p>";
    let content = extract_main_content(html);
    // Should not crash, returns something
    assert!(!content.is_empty());
}

// ── Metadata Extraction ──

#[test]
fn extract_metadata_empty_html() {
    let meta = extract_metadata("");
    assert!(meta.title.is_none());
    assert!(meta.description.is_none());
    assert!(meta.og_title.is_none());
    assert!(meta.og_description.is_none());
    assert!(meta.og_image.is_none());
    assert!(meta.canonical_url.is_none());
    assert!(meta.language.is_none());
}

#[test]
fn extract_metadata_populated() {
    let html = r#"<html lang="en">
        <head>
            <title>Test Page</title>
            <meta name="description" content="A test page">
            <meta property="og:title" content="OG Test">
            <meta property="og:description" content="OG Desc">
            <meta property="og:image" content="https://img.com/pic.jpg">
            <link rel="canonical" href="https://example.com/canonical">
        </head>
        <body></body>
    </html>"#;

    let meta = extract_metadata(html);
    assert_eq!(meta.title.as_deref(), Some("Test Page"));
    assert_eq!(meta.description.as_deref(), Some("A test page"));
    assert_eq!(meta.og_title.as_deref(), Some("OG Test"));
    assert_eq!(meta.og_description.as_deref(), Some("OG Desc"));
    assert_eq!(
        meta.og_image.as_deref(),
        Some("https://img.com/pic.jpg")
    );
    assert_eq!(
        meta.canonical_url.as_deref(),
        Some("https://example.com/canonical")
    );
    assert_eq!(meta.language.as_deref(), Some("en"));
}

// ── Link Extraction ──

#[test]
fn extract_links_relative_urls_resolved() {
    let html = r#"<html><body><a href="/page1">P1</a><a href="page2">P2</a></body></html>"#;
    let links = extract_links(html, "https://example.com/dir/");
    assert!(links.contains(&"https://example.com/page1".to_string()));
    assert!(links.contains(&"https://example.com/dir/page2".to_string()));
}

#[test]
fn extract_links_filters_fragment_only() {
    let html = r##"<html><body><a href="#section">Jump</a><a href="https://example.com">Real</a></body></html>"##;
    let links = extract_links(html, "https://example.com");
    assert_eq!(links.len(), 1);
    assert!(links[0].starts_with("https://example.com"));
}

#[test]
fn extract_links_filters_javascript_href() {
    let html = r#"<html><body><a href="javascript:void(0)">JS</a><a href="https://example.com">Real</a></body></html>"#;
    let links = extract_links(html, "https://example.com");
    assert_eq!(links.len(), 1);
}

#[test]
fn extract_links_filters_mailto() {
    let html = r#"<html><body><a href="mailto:test@example.com">Email</a><a href="https://example.com">Real</a></body></html>"#;
    let links = extract_links(html, "https://example.com");
    assert_eq!(links.len(), 1);
}

#[test]
fn extract_links_data_href_filtered() {
    let html = r#"<html><body><a href="data:text/html,<h1>XSS</h1>">Data</a><a href="https://example.com">Real</a></body></html>"#;
    let links = extract_links(html, "https://example.com");
    assert_eq!(links.len(), 1, "data: URIs should be filtered out");
    assert!(!links.iter().any(|l| l.starts_with("data:")));
}

#[test]
fn extract_links_tel_href_filtered() {
    let html = r#"<html><body><a href="tel:+1234567890">Call</a><a href="https://example.com">Real</a></body></html>"#;
    let links = extract_links(html, "https://example.com");
    assert_eq!(links.len(), 1, "tel: URIs should be filtered out");
}

#[test]
fn extract_links_blob_href_filtered() {
    let html = r#"<html><body><a href="blob:http://example.com/uuid">Blob</a><a href="https://example.com">Real</a></body></html>"#;
    let links = extract_links(html, "https://example.com");
    assert_eq!(links.len(), 1, "blob: URIs should be filtered out");
}

#[test]
fn extract_links_10k_links() {
    let mut html = String::from("<html><body>");
    for i in 0..10_000 {
        html.push_str(&format!(r#"<a href="/page{i}">Link {i}</a>"#));
    }
    html.push_str("</body></html>");

    let start = std::time::Instant::now();
    let links = extract_links(&html, "https://example.com");
    let elapsed = start.elapsed();

    assert_eq!(links.len(), 10_000);
    assert!(
        elapsed.as_secs() < 5,
        "10k links took too long: {elapsed:?}"
    );
}

#[test]
fn extract_links_no_anchors() {
    let html = "<html><body><p>No links here</p></body></html>";
    let links = extract_links(html, "https://example.com");
    assert!(links.is_empty());
}

#[test]
fn extract_links_invalid_base_url() {
    let html = r#"<html><body><a href="https://example.com">Link</a></body></html>"#;
    let links = extract_links(html, "not-a-url");
    // Absolute URLs should still work even with invalid base
    assert_eq!(links.len(), 1);
}
