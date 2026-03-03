use crw_extract::markdown::html_to_markdown;

#[test]
fn markdown_empty_input() {
    let result = html_to_markdown("");
    // Should not panic
    assert!(result.is_empty() || result.trim().is_empty() || !result.contains("<"));
}

#[test]
fn markdown_table_conversion() {
    let html = r#"<table><thead><tr><th>Name</th><th>Age</th></tr></thead><tbody><tr><td>Alice</td><td>30</td></tr></tbody></table>"#;
    let md = html_to_markdown(html);
    assert!(md.contains("Name"), "Table header missing. Got: {md}");
    assert!(md.contains("Alice"), "Table data missing. Got: {md}");
    assert!(md.contains("|"), "Should use pipe for table. Got: {md}");
}

#[test]
fn markdown_code_block() {
    let html = "<pre><code>fn main() {\n    println!(\"hello\");\n}</code></pre>";
    let md = html_to_markdown(html);
    assert!(
        md.contains("```") || md.contains("    fn main()"),
        "Code should be fenced or indented. Got: {md}"
    );
}

#[test]
fn markdown_heading_levels() {
    let html = "<h1>H1</h1><h2>H2</h2><h3>H3</h3><h4>H4</h4><h5>H5</h5><h6>H6</h6>";
    let md = html_to_markdown(html);
    assert!(md.contains("# H1"), "Missing h1. Got: {md}");
    assert!(md.contains("## H2"), "Missing h2. Got: {md}");
    assert!(md.contains("### H3"), "Missing h3. Got: {md}");
}

#[test]
fn markdown_1mb_document() {
    let paragraph = "<p>Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt.</p>\n";
    let count = 1_000_000 / paragraph.len() + 1;
    let html: String = paragraph.repeat(count);
    assert!(html.len() >= 1_000_000);

    let start = std::time::Instant::now();
    let md = html_to_markdown(&html);
    let elapsed = start.elapsed();

    assert!(!md.is_empty(), "Should produce markdown output");
    assert!(
        elapsed.as_secs() < 10,
        "1MB markdown conversion took too long: {elapsed:?}"
    );
}
