use crw_renderer::detector::{is_thin_markdown, looks_like_thin_html, needs_js_rendering};

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

#[test]
fn detector_nextjs_data_island_marker() {
    let html = r#"<html><body><div id="__next"></div><script id="__NEXT_DATA__">{}</script></body></html>"#;
    assert!(needs_js_rendering(html));
}

#[test]
fn detector_remix_marker() {
    let html = r#"<html><body data-remix-run><div></div></body></html>"#;
    assert!(needs_js_rendering(html));
}

#[test]
fn detector_sveltekit_marker() {
    let html = r#"<html><body><div data-sveltekit-preload-data></div></body></html>"#;
    assert!(needs_js_rendering(html));
}

#[test]
fn detector_astro_marker() {
    let html = r#"<html><body><div data-astro-cid-x12></div></body></html>"#;
    assert!(needs_js_rendering(html));
}

#[test]
fn detector_gatsby_marker() {
    let html = r#"<html><body><div id="gatsby-focus-wrapper"></div></body></html>"#;
    assert!(needs_js_rendering(html));
}

#[test]
fn detector_short_body_many_scripts() {
    // Modern bundler-heavy SPA without recognizable framework marker:
    // 5+ script tags + thin body.
    let html = r#"<html><body><div></div>
        <script src="/a.js"></script>
        <script src="/b.js"></script>
        <script src="/c.js"></script>
        <script src="/d.js"></script>
        <script src="/e.js"></script>
    </body></html>"#;
    assert!(needs_js_rendering(html));
}

#[test]
fn detector_three_scripts_not_enough_for_spa() {
    // Threshold for the bundler-heavy SPA branch is 5+ scripts. A minimal
    // page with just the standard analytics+ads+font scripts must NOT trip
    // it. Body text is kept above 200 to bypass the `<script src` SPA-marker
    // branch (that branch is gated on body_len<200 with an indicator).
    let body_text = "Hello world. ".repeat(20);
    let html = format!(
        r#"<html><body><p>{body_text}</p>
        <script src="/ga.js"></script>
        <script src="/ads.js"></script>
        <script src="/font.js"></script>
    </body></html>"#
    );
    assert!(!needs_js_rendering(&html));
}

#[test]
fn detector_long_body_many_scripts_not_spa() {
    // A real article with analytics scripts must not be flagged.
    let body_text = "Real journalism content. ".repeat(80);
    let html = format!(
        r#"<html><body><article>{body_text}</article>
        <script src="/ga.js"></script>
        <script src="/ads.js"></script>
        <script src="/track.js"></script>
        <script src="/cookie.js"></script>
        <script src="/abtest.js"></script>
        </body></html>"#
    );
    assert!(!needs_js_rendering(&html));
}

#[test]
fn thin_html_short_body_triggers() {
    let html = r#"<html><body><div></div></body></html>"#;
    assert!(looks_like_thin_html(html));
}

#[test]
fn thin_html_substantive_page_not_thin() {
    let body_text = "Real article content. ".repeat(80);
    let html = format!("<html><body><article>{body_text}</article></body></html>");
    assert!(!looks_like_thin_html(&html));
}

#[test]
fn thin_markdown_floor_is_100_bytes() {
    assert!(is_thin_markdown(0));
    assert!(is_thin_markdown(50));
    assert!(is_thin_markdown(99));
    assert!(!is_thin_markdown(100));
    assert!(!is_thin_markdown(5000));
}
