use crw_extract::clean::clean_html;

// ── P0: Basic edge cases ──

#[test]
fn clean_html_empty_input() {
    let result = clean_html("", false, &[], &[]).unwrap();
    assert!(result.is_empty() || result.trim().is_empty());
}

#[test]
fn clean_html_no_tags() {
    let result = clean_html("Hello world", false, &[], &[]).unwrap();
    assert!(result.contains("Hello world"));
}

#[test]
fn clean_html_nested_scripts() {
    let html =
        r#"<body><script>var x = "<script>alert(1)</script>";</script><p>Content</p></body>"#;
    let result = clean_html(html, false, &[], &[]).unwrap();
    assert!(!result.contains("alert"));
    assert!(result.contains("Content"));
}

#[test]
fn clean_html_malformed_unclosed_tags() {
    let html = "<body><p>Hello<div>World</body>";
    // Should not crash
    let result = clean_html(html, false, &[], &[]);
    assert!(result.is_ok());
}

// ── P1: Unicode and special content ──

#[test]
fn clean_html_unicode_content() {
    let html = "<body><p>Türkçe İçerik</p><p>日本語テスト</p><p>🚀🎉</p></body>";
    let result = clean_html(html, false, &[], &[]).unwrap();
    assert!(result.contains("Türkçe İçerik"));
    assert!(result.contains("日本語テスト"));
    assert!(result.contains("🚀🎉"));
}

#[test]
fn clean_html_invalid_css_selector() {
    // Invalid selector should warn but not crash
    let html = "<body><p>Content</p></body>";
    let result = clean_html(html, false, &["[[[invalid".into()], &[]).unwrap();
    // Falls back to original since no valid selector matched
    assert!(result.contains("Content"));
}

#[test]
fn clean_html_self_closing_tags_preserved() {
    let html = "<body><p>Before</p><br><hr><img src=\"x\"><p>After</p></body>";
    let result = clean_html(html, false, &[], &[]).unwrap();
    assert!(result.contains("Before"));
    assert!(result.contains("After"));
}

// ── P0: Chaos tests - Malformed HTML ──

#[test]
fn clean_html_deeply_nested_divs() {
    // 1000 nested divs (reduced from 10000 for test speed, but still tests depth handling)
    let open_tags: String = (0..1000).map(|_| "<div>").collect();
    let close_tags: String = (0..1000).map(|_| "</div>").collect();
    let html = format!("<body>{open_tags}Content{close_tags}</body>");
    let result = clean_html(&html, false, &[], &[]);
    assert!(
        result.is_ok(),
        "Should not OOM or crash on deeply nested HTML"
    );
}

#[test]
fn clean_html_null_bytes() {
    let html = "<body><p>Hello\0World</p></body>";
    let result = clean_html(html, false, &[], &[]);
    // Should not crash — may or may not preserve the null byte
    assert!(result.is_ok());
}

#[test]
fn clean_html_huge_single_tag() {
    // 100KB attribute value
    let attr_val = "x".repeat(100_000);
    let html = format!(r#"<body><div data-test="{attr_val}"><p>Content</p></div></body>"#);
    let result = clean_html(&html, false, &[], &[]);
    assert!(result.is_ok());
}

// ── P0: Performance - Large documents ──

#[test]
fn clean_html_1mb_document() {
    let paragraph = "<p>Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.</p>\n";
    let count = 1_000_000 / paragraph.len() + 1;
    let body: String = paragraph.repeat(count);
    let html = format!("<html><body>{body}</body></html>");
    assert!(html.len() >= 1_000_000);

    let start = std::time::Instant::now();
    let result = clean_html(&html, false, &[], &[]);
    let elapsed = start.elapsed();

    assert!(result.is_ok(), "Should handle 1MB document");
    assert!(
        elapsed.as_secs() < 5,
        "1MB document took too long: {elapsed:?}"
    );
}

// ── Removal tests ──

#[test]
fn clean_html_removes_all_unwanted_tags() {
    let html = r#"<body>
        <script>alert(1)</script>
        <style>.x{color:red}</style>
        <noscript>Enable JS</noscript>
        <iframe src="evil.html"></iframe>
        <svg><rect/></svg>
        <canvas></canvas>
        <p>Content</p>
    </body>"#;
    let result = clean_html(html, false, &[], &[]).unwrap();
    assert!(!result.contains("<script"));
    assert!(!result.contains("<style"));
    assert!(!result.contains("<noscript"));
    assert!(!result.contains("<iframe"));
    assert!(!result.contains("<svg"));
    assert!(!result.contains("<canvas"));
    assert!(result.contains("Content"));
}

#[test]
fn clean_html_main_content_mode_removes_extras() {
    let html = "<body><nav>Nav</nav><header>Head</header><aside>Side</aside><menu>Menu</menu><article>Content</article><footer>Foot</footer></body>";
    let result = clean_html(html, true, &[], &[]).unwrap();
    assert!(!result.contains("Nav"));
    assert!(!result.contains("Head"));
    assert!(!result.contains("Side"));
    assert!(!result.contains("Menu"));
    assert!(!result.contains("Foot"));
    assert!(result.contains("Content"));
}

#[test]
fn clean_html_removes_data_uri_images() {
    let html =
        r#"<body><img src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUg..."><p>Content</p></body>"#;
    let result = clean_html(html, false, &[], &[]).unwrap();
    assert!(
        !result.contains("data:image"),
        "data: URI images should be removed"
    );
    assert!(result.contains("Content"));
}

#[test]
fn clean_html_preserves_normal_images() {
    let html = r#"<body><img src="https://example.com/photo.jpg"><p>Content</p></body>"#;
    let result = clean_html(html, false, &[], &[]).unwrap();
    assert!(
        result.contains("photo.jpg"),
        "Normal images should be preserved"
    );
}

#[test]
fn clean_html_removes_select_elements_in_main_content_mode() {
    let html = r#"<body><select><option>Istanbul</option><option>Ankara</option></select><p>Content</p></body>"#;
    let result = clean_html(html, true, &[], &[]).unwrap();
    assert!(
        !result.contains("Istanbul"),
        "Select elements should be removed in main content mode"
    );
    assert!(result.contains("Content"));
}

#[test]
fn clean_html_removes_dropdown_noise_class() {
    let html = r#"<body><div class="dropdown">Dropdown content</div><p>Content</p></body>"#;
    let result = clean_html(html, true, &[], &[]).unwrap();
    assert!(
        !result.contains("Dropdown content"),
        "Dropdown class should be noise-removed"
    );
    assert!(result.contains("Content"));
}
