use crw_core::types::OutputFormat;
use crw_extract::ExtractOptions;

#[test]
fn extract_markdown_format() {
    let html = "<html><head><title>Test</title></head><body><article><h1>Hello</h1><p>World</p></article></body></html>";
    let data = crw_extract::extract(ExtractOptions {
        raw_html: html,
        source_url: "https://example.com",
        status_code: 200,
        rendered_with: None,
        elapsed_ms: 100,
        render_decision: None,
        credit_cost: 0,
        warnings: Vec::new(),
        formats: &[OutputFormat::Markdown],
        only_main_content: true,
        include_tags: &[],
        exclude_tags: &[],
        css_selector: None,
        xpath: None,
        chunk_strategy: None,
        query: None,
        filter_mode: None,
        top_k: None,
        domain_selectors: None,
        captured_responses: &[],
        llm_fallback: None,
        debug: false,
        debug_sink: None,
    })
    .unwrap();

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

    let data = crw_extract::extract(ExtractOptions {
        raw_html: html,
        source_url: "https://example.com",
        status_code: 200,
        rendered_with: Some("http".into()),
        elapsed_ms: 50,
        render_decision: None,
        credit_cost: 0,
        warnings: Vec::new(),
        formats: &formats,
        only_main_content: false,
        include_tags: &[],
        exclude_tags: &[],
        css_selector: None,
        xpath: None,
        chunk_strategy: None,
        query: None,
        filter_mode: None,
        top_k: None,
        domain_selectors: None,
        captured_responses: &[],
        llm_fallback: None,
        debug: false,
        debug_sink: None,
    })
    .unwrap();

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

    let data = crw_extract::extract(ExtractOptions {
        raw_html: html,
        source_url: "https://example.com",
        status_code: 200,
        rendered_with: None,
        elapsed_ms: 10,
        render_decision: None,
        credit_cost: 0,
        warnings: Vec::new(),
        formats: &[OutputFormat::Markdown],
        only_main_content: false,
        include_tags: &[],
        exclude_tags: &[],
        css_selector: None,
        xpath: None,
        chunk_strategy: None,
        query: None,
        filter_mode: None,
        top_k: None,
        domain_selectors: None,
        captured_responses: &[],
        llm_fallback: None,
        debug: false,
        debug_sink: None,
    })
    .unwrap();

    assert_eq!(data.metadata.title.as_deref(), Some("My Page"));
    assert_eq!(data.metadata.description.as_deref(), Some("A description"));
    assert_eq!(data.metadata.language.as_deref(), Some("en"));
}

#[test]
fn extract_empty_html() {
    let data = crw_extract::extract(ExtractOptions {
        raw_html: "",
        source_url: "https://example.com",
        status_code: 200,
        rendered_with: None,
        elapsed_ms: 0,
        render_decision: None,
        credit_cost: 0,
        warnings: Vec::new(),
        formats: &[OutputFormat::Markdown, OutputFormat::PlainText],
        only_main_content: false,
        include_tags: &[],
        exclude_tags: &[],
        css_selector: None,
        xpath: None,
        chunk_strategy: None,
        query: None,
        filter_mode: None,
        top_k: None,
        domain_selectors: None,
        captured_responses: &[],
        llm_fallback: None,
        debug: false,
        debug_sink: None,
    })
    .unwrap();

    // Should not crash
    assert!(data.markdown.is_some());
    assert!(data.plain_text.is_some());
}

#[test]
fn extract_with_include_exclude_tags() {
    let html =
        r#"<html><body><div class="ad">Ad</div><article><p>Content</p></article></body></html>"#;
    let data = crw_extract::extract(ExtractOptions {
        raw_html: html,
        source_url: "https://example.com",
        status_code: 200,
        rendered_with: None,
        elapsed_ms: 0,
        render_decision: None,
        credit_cost: 0,
        warnings: Vec::new(),
        formats: &[OutputFormat::Markdown],
        only_main_content: false,
        include_tags: &["article".into()],
        exclude_tags: &[],
        css_selector: None,
        xpath: None,
        chunk_strategy: None,
        query: None,
        filter_mode: None,
        top_k: None,
        domain_selectors: None,
        captured_responses: &[],
        llm_fallback: None,
        debug: false,
        debug_sink: None,
    })
    .unwrap();

    let md = data.markdown.unwrap();
    assert!(md.contains("Content"), "Should include article content");
}

/// News/blog templates often place the article H1 in a `<header>` sibling of
/// the readability-scored container, so the title vanishes from markdown.
/// `extract` must restore it from the metadata title (preferring `og:title`).
#[test]
fn prepends_metadata_title_when_missing_from_markdown() {
    let html = r#"<html><head>
        <title>Compute Module 4 Cold Spec - Raspberry Pi</title>
        <meta property="og:title" content="New extended temperature range for Compute Module 4">
    </head><body>
        <header><h1>New extended temperature range for Compute Module 4</h1></header>
        <article><p>Body paragraph that mentions thousands of embedded customers in challenging environments.</p></article>
    </body></html>"#;
    let data = crw_extract::extract(ExtractOptions {
        raw_html: html,
        source_url: "https://example.com",
        status_code: 200,
        rendered_with: None,
        elapsed_ms: 0,
        render_decision: None,
        credit_cost: 0,
        warnings: Vec::new(),
        formats: &[OutputFormat::Markdown],
        only_main_content: true,
        include_tags: &[],
        exclude_tags: &[],
        css_selector: None,
        xpath: None,
        chunk_strategy: None,
        query: None,
        filter_mode: None,
        top_k: None,
        domain_selectors: None,
        captured_responses: &[],
        llm_fallback: None,
        debug: false,
        debug_sink: None,
    })
    .unwrap();

    let md = data.markdown.unwrap();
    assert!(
        md.contains("New extended temperature range for Compute Module 4"),
        "title should be present in markdown, got: {md:?}"
    );
}

/// When the title is already present in the extracted markdown (e.g. the H1
/// lived inside the readability-selected article), don't double up.
#[test]
fn does_not_duplicate_title_already_in_markdown() {
    let html = r#"<html><head><title>Hello World</title></head><body>
        <article><h1>Hello World</h1><p>Body paragraph that gives readability some content to score.</p></article>
    </body></html>"#;
    let data = crw_extract::extract(ExtractOptions {
        raw_html: html,
        source_url: "https://example.com",
        status_code: 200,
        rendered_with: None,
        elapsed_ms: 0,
        render_decision: None,
        credit_cost: 0,
        warnings: Vec::new(),
        formats: &[OutputFormat::Markdown],
        only_main_content: true,
        include_tags: &[],
        exclude_tags: &[],
        css_selector: None,
        xpath: None,
        chunk_strategy: None,
        query: None,
        filter_mode: None,
        top_k: None,
        domain_selectors: None,
        captured_responses: &[],
        llm_fallback: None,
        debug: false,
        debug_sink: None,
    })
    .unwrap();

    let md = data.markdown.unwrap();
    assert_eq!(
        md.matches("Hello World").count(),
        1,
        "title should appear exactly once, got: {md:?}"
    );
}

/// Strip " | Site Name" / " - Site Name" / em-dash suffixes from raw `<title>`.
#[test]
fn strips_site_name_suffix_from_title_when_prepending() {
    let html = r#"<html><head>
        <title>Article Title – Some Blog</title>
    </head><body>
        <article><p>Body content paragraph for readability to chew on.</p></article>
    </body></html>"#;
    let data = crw_extract::extract(ExtractOptions {
        raw_html: html,
        source_url: "https://example.com",
        status_code: 200,
        rendered_with: None,
        elapsed_ms: 0,
        render_decision: None,
        credit_cost: 0,
        warnings: Vec::new(),
        formats: &[OutputFormat::Markdown],
        only_main_content: true,
        include_tags: &[],
        exclude_tags: &[],
        css_selector: None,
        xpath: None,
        chunk_strategy: None,
        query: None,
        filter_mode: None,
        top_k: None,
        domain_selectors: None,
        captured_responses: &[],
        llm_fallback: None,
        debug: false,
        debug_sink: None,
    })
    .unwrap();

    let md = data.markdown.unwrap();
    assert!(
        md.contains("Article Title"),
        "core title should appear: {md:?}"
    );
    assert!(
        !md.contains("Some Blog"),
        "site-name suffix should be stripped: {md:?}"
    );
}

/// Regression: the title-suffix stripper must not split on bare en/em dashes
/// without surrounding whitespace. metmuseum's `<title>` is
/// "Northern Song Dynasty (960–1127) | Essay | …" — splitting on a bare en
/// dash truncated the title to "Northern Song Dynasty (960", which then no
/// longer matched any body phrase. Whitespace-anchored splits preserve the
/// in-title dash.
#[test]
fn preserves_en_dash_inside_title_parentheses() {
    let html = r#"<html><head>
        <title>Northern Song Dynasty (960–1127) | Essay | The Met</title>
    </head><body>
        <article><p>The Song dynasty was a brilliant era in Chinese history with substantial cultural achievement across the centuries.</p></article>
    </body></html>"#;
    let data = crw_extract::extract(ExtractOptions {
        raw_html: html,
        source_url: "https://example.com",
        status_code: 200,
        rendered_with: None,
        elapsed_ms: 0,
        render_decision: None,
        credit_cost: 0,
        warnings: Vec::new(),
        formats: &[OutputFormat::Markdown],
        only_main_content: true,
        include_tags: &[],
        exclude_tags: &[],
        css_selector: None,
        xpath: None,
        chunk_strategy: None,
        query: None,
        filter_mode: None,
        top_k: None,
        domain_selectors: None,
        captured_responses: &[],
        llm_fallback: None,
        debug: false,
        debug_sink: None,
    })
    .unwrap();

    let md = data.markdown.unwrap();
    assert!(
        md.contains("Northern Song Dynasty (960–1127)"),
        "in-title en dash must survive: {md:?}"
    );
    assert!(
        !md.contains("The Met"),
        "site-name suffix after pipe should be stripped: {md:?}"
    );
}

/// When the caller passed an explicit selector, the user opted into a narrow
/// extraction — we must not inject metadata they didn't ask for.
#[test]
fn does_not_prepend_title_when_css_selector_provided() {
    let html = r#"<html><head>
        <meta property="og:title" content="Page Title">
    </head><body>
        <main><p id="target">Just this paragraph.</p></main>
    </body></html>"#;
    let data = crw_extract::extract(ExtractOptions {
        raw_html: html,
        source_url: "https://example.com",
        status_code: 200,
        rendered_with: None,
        elapsed_ms: 0,
        render_decision: None,
        credit_cost: 0,
        warnings: Vec::new(),
        formats: &[OutputFormat::Markdown],
        only_main_content: true,
        include_tags: &[],
        exclude_tags: &[],
        css_selector: Some("#target"),
        xpath: None,
        chunk_strategy: None,
        query: None,
        filter_mode: None,
        top_k: None,
        domain_selectors: None,
        captured_responses: &[],
        llm_fallback: None,
        debug: false,
        debug_sink: None,
    })
    .unwrap();

    let md = data.markdown.unwrap();
    assert!(
        !md.contains("Page Title"),
        "selector path must not inject metadata title: {md:?}"
    );
}

/// A *domain*-config-supplied selector (auto-applied per host) is not user
/// opt-in — title prepending must still fire when the article H1 lives outside
/// the selected container. Regression: `www.raspberrypi.com` ships with the
/// default selector `article.entry-content, main`; without this carve-out, the
/// title injection silently skips for every domain in `[extraction.domain_selectors]`.
#[test]
fn prepends_title_when_only_domain_selector_applies() {
    let html = r#"<html><head>
        <meta property="og:title" content="New extended temperature range for Compute Module 4 - Raspberry Pi">
    </head><body>
        <nav><h1>News</h1></nav>
        <main><p>While the Raspberry Pi project has its origins in education, the majority of Raspberry Pi computers we make today are destined for industrial and embedded applications.</p></main>
    </body></html>"#;
    let mut domain_map = std::collections::HashMap::new();
    domain_map.insert("www.raspberrypi.com".to_string(), "main".to_string());
    let data = crw_extract::extract(ExtractOptions {
        raw_html: html,
        source_url: "https://www.raspberrypi.com/news/x/",
        status_code: 200,
        rendered_with: None,
        elapsed_ms: 0,
        render_decision: None,
        credit_cost: 0,
        warnings: Vec::new(),
        formats: &[OutputFormat::Markdown],
        only_main_content: true,
        include_tags: &[],
        exclude_tags: &[],
        css_selector: None,
        xpath: None,
        chunk_strategy: None,
        query: None,
        filter_mode: None,
        top_k: None,
        domain_selectors: Some(&domain_map),
        captured_responses: &[],
        llm_fallback: None,
        debug: false,
        debug_sink: None,
    })
    .unwrap();

    let md = data.markdown.unwrap();
    assert!(
        md.contains("New extended temperature range for Compute Module 4"),
        "domain-default selector must not suppress title prepend: {md:?}"
    );
}
