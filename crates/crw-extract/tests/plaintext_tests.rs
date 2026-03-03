use crw_extract::plaintext::html_to_plaintext;

#[test]
fn plaintext_collapses_multiple_spaces() {
    let html = "<html><body><p>Hello     world</p></body></html>";
    let text = html_to_plaintext(html);
    assert!(
        text.contains("Hello world"),
        "Should collapse spaces. Got: '{text}'"
    );
    assert!(
        !text.contains("     "),
        "Should not have 5 consecutive spaces"
    );
}

#[test]
fn plaintext_preserves_newlines() {
    let html = "<html><body><p>Line 1</p><p>Line 2</p></body></html>";
    let text = html_to_plaintext(html);
    assert!(text.contains("Line 1"));
    assert!(text.contains("Line 2"));
}

#[test]
fn plaintext_empty_html() {
    let text = html_to_plaintext("");
    assert!(text.is_empty() || text.trim().is_empty());
}

#[test]
fn plaintext_strips_all_tags() {
    let html = "<html><body><h1>Title</h1><p>Content with <strong>bold</strong> and <a href='x'>link</a></p></body></html>";
    let text = html_to_plaintext(html);
    assert!(text.contains("Title"));
    assert!(text.contains("Content with"));
    assert!(text.contains("bold"));
    assert!(text.contains("link"));
    assert!(!text.contains("<"));
}
