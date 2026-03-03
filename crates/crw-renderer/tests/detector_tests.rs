use crw_renderer::detector::needs_js_rendering;

#[test]
fn detector_empty_body_with_root_div() {
    let html = r#"<html><head></head><body><div id="root"></div><script src="/app.js"></script></body></html>"#;
    assert!(needs_js_rendering(html), "Empty body + root div = SPA");
}

#[test]
fn detector_noscript_enable_javascript() {
    let html = r#"<html><body><noscript>Please enable JavaScript to continue</noscript><div id="app"></div></body></html>"#;
    assert!(needs_js_rendering(html));
}

#[test]
fn detector_body_text_above_threshold_with_indicators() {
    // Body has >100 chars of text content, even with SPA indicators — should NOT trigger
    let long_text = "A".repeat(200);
    let html = format!(
        r#"<html><body><div id="root"><p>{long_text}</p></div><script src="/app.js"></script></body></html>"#
    );
    assert!(!needs_js_rendering(&html), "Long body text = not SPA shell");
}

#[test]
fn detector_minimal_html_no_body() {
    // No <body> tag at all — extract_body_text_len returns 1000, so >100 threshold
    let html = "<html><head></head></html>";
    assert!(!needs_js_rendering(html), "No body tag should return false");
}

#[test]
fn detector_nuxt_app() {
    let html = r#"<html><head></head><body><div id="__nuxt"></div><script src="/nuxt.js"></script></body></html>"#;
    assert!(
        needs_js_rendering(html),
        "Nuxt app marker should detect SPA"
    );
}

#[test]
fn detector_next_app() {
    let html = r#"<html><head></head><body><div id="__next"></div><script src="/next.js"></script></body></html>"#;
    assert!(
        needs_js_rendering(html),
        "Next.js app marker should detect SPA"
    );
}

#[test]
fn detector_react_root() {
    let html = r#"<html><head></head><body><div data-reactroot></div><script src="/react.js"></script></body></html>"#;
    assert!(needs_js_rendering(html));
}

#[test]
fn detector_angular_ng_app() {
    let html = r#"<html ng-app="myApp"><head></head><body><div></div></body></html>"#;
    assert!(needs_js_rendering(html));
}

#[test]
fn detector_static_page_plenty_of_text() {
    let long_article = "This is a long article with lots of text. ".repeat(10);
    let html = format!("<html><body><article><p>{long_article}</p></article></body></html>");
    assert!(!needs_js_rendering(&html));
}

#[test]
fn detector_only_checks_first_50kb() {
    // Put a SPA indicator after 50KB — should NOT be detected
    let padding = "x".repeat(60_000);
    let html = format!(
        r#"<html><body><p>{padding}</p><div id="root"></div><script src="/app.js"></script></body></html>"#
    );
    // The body text is >100 chars AND the indicator is after 50KB, so should not trigger
    assert!(!needs_js_rendering(&html));
}
