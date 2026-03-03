use crw_crawl::robots::RobotsTxt;

#[test]
fn robots_multiple_user_agents() {
    let text = r#"
User-agent: Googlebot
Disallow: /google-only/

User-agent: *
Disallow: /admin/
Disallow: /private/

User-agent: Bingbot
Disallow: /bing-only/

Sitemap: https://example.com/sitemap.xml
"#;
    let robots = RobotsTxt::parse(text);
    // Only * section should apply to CRW
    assert!(!robots.is_allowed("/admin/page"));
    assert!(!robots.is_allowed("/private/page"));
    // Googlebot-specific rules should NOT apply
    assert!(robots.is_allowed("/google-only/page"));
    // Bingbot-specific rules should NOT apply
    assert!(robots.is_allowed("/bing-only/page"));
    assert_eq!(robots.sitemaps, vec!["https://example.com/sitemap.xml"]);
}

#[test]
fn robots_empty_file() {
    let robots = RobotsTxt::parse("");
    assert!(robots.is_allowed("/anything"));
    assert!(robots.is_allowed("/admin/secret"));
    assert!(robots.sitemaps.is_empty());
}

#[test]
fn robots_crw_specific_section() {
    let text = r#"
User-agent: *
Disallow: /

User-agent: CRW
Disallow: /crw-blocked/
"#;
    let robots = RobotsTxt::parse(text);
    // Both * and CRW sections apply, so / is disallowed from * section
    assert!(!robots.is_allowed("/anything"));
    assert!(!robots.is_allowed("/crw-blocked/page"));
}

#[test]
fn robots_wildcard_in_disallow() {
    let text = r#"
User-agent: *
Disallow: /*.pdf
"#;
    let robots = RobotsTxt::parse(text);
    assert!(!robots.is_allowed("/document.pdf"), "Wildcard patterns should block .pdf URLs");
    assert!(!robots.is_allowed("/path/to/file.pdf"));
    assert!(robots.is_allowed("/document.html"));
}

#[test]
fn robots_allow_directive() {
    let text = r#"
User-agent: *
Disallow: /private/
Allow: /private/public-page
"#;
    let robots = RobotsTxt::parse(text);
    assert!(!robots.is_allowed("/private/secret"));
    assert!(robots.is_allowed("/private/public-page"));
    assert!(robots.is_allowed("/public/anything"));
}

#[test]
fn robots_specificity_longer_wins() {
    let text = r#"
User-agent: *
Disallow: /
Allow: /public/
"#;
    let robots = RobotsTxt::parse(text);
    assert!(!robots.is_allowed("/private"));
    assert!(robots.is_allowed("/public/page"));
}

#[test]
fn robots_dollar_end_anchor() {
    let text = "User-agent: *\nDisallow: /*.pdf$\n";
    let robots = RobotsTxt::parse(text);
    assert!(!robots.is_allowed("/doc.pdf"));
    assert!(robots.is_allowed("/doc.pdf?q=1"));
}

#[test]
fn robots_comments_ignored() {
    // Note: inline comments (after directives) are NOT stripped by the parser.
    // Only full-line comments (starting with #) are properly handled.
    let text = r#"
# This is a comment
User-agent: *
Disallow: /blocked/
# Another comment
Sitemap: https://example.com/sitemap.xml
"#;
    let robots = RobotsTxt::parse(text);
    assert!(!robots.is_allowed("/blocked/page"));
    assert!(robots.is_allowed("/allowed/page"));
}

#[test]
fn robots_multiple_sitemaps() {
    let text = r#"
User-agent: *
Disallow:

Sitemap: https://example.com/sitemap1.xml
Sitemap: https://example.com/sitemap2.xml
Sitemap: https://example.com/sitemap3.xml
"#;
    let robots = RobotsTxt::parse(text);
    assert_eq!(robots.sitemaps.len(), 3);
    assert!(robots.is_allowed("/anything"));
}

#[test]
fn robots_case_insensitive_directives() {
    let text = "USER-AGENT: *\nDISALLOW: /blocked/\nSITEMAP: https://example.com/sitemap.xml\n";
    let robots = RobotsTxt::parse(text);
    assert!(!robots.is_allowed("/blocked/page"));
    assert_eq!(robots.sitemaps.len(), 1);
}
