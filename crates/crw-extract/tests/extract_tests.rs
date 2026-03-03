use crw_core::types::OutputFormat;

#[test]
fn extract_markdown_format() {
    let html = "<html><head><title>Test</title></head><body><article><h1>Hello</h1><p>World</p></article></body></html>";
    let data = crw_extract::extract(
        html,
        "https://example.com",
        200,
        None,
        100,
        &[OutputFormat::Markdown],
        true,
        &[],
        &[],
    );

    assert!(data.markdown.is_some());
    assert!(data.html.is_none());
    assert!(data.raw_html.is_none());
    assert!(data.plain_text.is_none());
    assert!(data.links.is_none());
    assert!(data.json.is_none());
    assert_eq!(data.metadata.status_code, 200);
    assert_eq!(data.metadata.source_url, "https://example.com");
}

#[test]
fn extract_all_formats() {
    let html = "<html><head><title>Test</title></head><body><article><h1>Hello</h1><p>World</p><a href='/page'>Link</a></article></body></html>";
    let formats = vec![
        OutputFormat::Markdown,
        OutputFormat::Html,
        OutputFormat::RawHtml,
        OutputFormat::PlainText,
        OutputFormat::Links,
    ];

    let data = crw_extract::extract(
        html,
        "https://example.com",
        200,
        Some("http".into()),
        50,
        &formats,
        false,
        &[],
        &[],
    );

    assert!(data.markdown.is_some(), "markdown should be present");
    assert!(data.html.is_some(), "html should be present");
    assert!(data.raw_html.is_some(), "raw_html should be present");
    assert!(data.plain_text.is_some(), "plain_text should be present");
    assert!(data.links.is_some(), "links should be present");
    // JSON is always None from extract() — handled async separately
    assert!(data.json.is_none());

    assert_eq!(data.metadata.rendered_with.as_deref(), Some("http"));
    assert_eq!(data.metadata.elapsed_ms, 50);
}

#[test]
fn extract_metadata_populated() {
    let html = r#"<html lang="en"><head>
        <title>My Page</title>
        <meta name="description" content="A description">
    </head><body><p>Content</p></body></html>"#;

    let data = crw_extract::extract(
        html,
        "https://example.com",
        200,
        None,
        10,
        &[OutputFormat::Markdown],
        false,
        &[],
        &[],
    );

    assert_eq!(data.metadata.title.as_deref(), Some("My Page"));
    assert_eq!(data.metadata.description.as_deref(), Some("A description"));
    assert_eq!(data.metadata.language.as_deref(), Some("en"));
}

#[test]
fn extract_empty_html() {
    let data = crw_extract::extract(
        "",
        "https://example.com",
        200,
        None,
        0,
        &[OutputFormat::Markdown, OutputFormat::PlainText],
        false,
        &[],
        &[],
    );

    // Should not crash
    assert!(data.markdown.is_some());
    assert!(data.plain_text.is_some());
}

#[test]
fn extract_with_include_exclude_tags() {
    let html =
        r#"<html><body><div class="ad">Ad</div><article><p>Content</p></article></body></html>"#;
    let data = crw_extract::extract(
        html,
        "https://example.com",
        200,
        None,
        0,
        &[OutputFormat::Markdown],
        false,
        &["article".into()],
        &[],
    );

    let md = data.markdown.unwrap();
    assert!(md.contains("Content"), "Should include article content");
}
