use crw_extract::readability::{extract_links, extract_main_content, extract_metadata};

// ── Main Content Extraction ──

#[test]
fn broad_main_finds_inner_content() {
    let filler = "x".repeat(500);
    let html = format!(
        r#"<html><body>
        <main>
            <nav>Navigation {filler}</nav>
            <div class="main-page-content">
                <h1>Promise</h1>
                <p>The Promise object represents the eventual completion. {filler}</p>
            </div>
            <footer>Footer {filler}</footer>
        </main>
    </body></html>"#
    );
    let content = extract_main_content(&html);
    assert!(
        content.contains("Promise"),
        "Should find inner content. Got: {content}"
    );
    assert!(!content.contains("Navigation"), "Should not include nav");
}

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

#[test]
fn picks_content_main_over_filter_main() {
    let body = "x".repeat(800);
    let html = format!(
        r#"<html><body>
        <main class="filter-pane"><p>Sort by date</p><p>Distance</p><p>JobType</p></main>
        <main class="content"><article><h1>Real article</h1><p>Long body about the topic. {body}</p></article></main>
        </body></html>"#
    );
    let content = extract_main_content(&html);
    assert!(
        content.contains("Real article"),
        "Should contain real article. Got: {content}"
    );
    assert!(
        !content.contains("JobType"),
        "Should not include filter pane. Got: {content}"
    );
}

#[test]
fn picks_article_over_navigation_main() {
    let body = "y".repeat(600);
    let html = format!(
        r#"<html><body>
        <article><h1>Title</h1><p>Article body content here. {body}</p></article>
        <main role="navigation"><a href="/">Home</a><a href="/about">About</a><a href="/contact">Contact</a></main>
        </body></html>"#
    );
    let content = extract_main_content(&html);
    assert!(
        content.contains("Article body content"),
        "Should contain article. Got: {content}"
    );
    assert!(
        !content.contains("About"),
        "Should not include nav links. Got: {content}"
    );
}

#[test]
fn single_main_whole_body_unchanged() {
    let body = "z".repeat(800);
    let html = format!(
        r#"<html><body><main><h1>Title</h1><p>Whole document body. {body}</p></main></body></html>"#
    );
    let content = extract_main_content(&html);
    assert!(
        content.contains("Whole document body"),
        "Should still extract single-main content. Got: {content}"
    );
}

#[test]
fn simplyhired_style_picks_listings() {
    let html = r#"<html><body>
        <main>
            <aside class="filter"><p>Sort</p><p>Distance</p><p>JobType</p><p>Salary</p><p>Location</p></aside>
            <section class="results">
                <article><h2>Job 1: Senior Biostatistician at MIT</h2><p>Description of the role and responsibilities for this senior biostatistics position. Requires expertise in clinical trials, R, SAS, and statistical modeling at scale across teams and departments.</p></article>
                <article><h2>Job 2: Junior Biostatistician at Harvard</h2><p>Another long description of responsibilities and qualifications needed for this entry-level biostatistics role focused on data cleaning, exploratory analysis, and supporting senior staff.</p></article>
                <article><h2>Job 3: Research Biostatistician</h2><p>Yet another posting with details about compensation, benefits, and requirements for a research-focused biostatistics role at a university medical center with collaborative research opportunities.</p></article>
            </section>
        </main>
    </body></html>"#;
    let content = extract_main_content(html);
    assert!(
        content.contains("Senior Biostatistician"),
        "Should contain job listings. Got: {content}"
    );
    assert!(
        !content.contains("Distance"),
        "Should not contain filter aside. Got: {content}"
    );
}

#[test]
fn velou_style_includes_hero() {
    let html = r#"<html><body>
        <main>
            <div class="hero"><h1>Win with Velou in the Agent-first World</h1><p>The agent-first world is here and Velou is leading the charge with new tools.</p></div>
            <div class="logos"><a href="/c1"><img src="c1.png"></a><a href="/c2"><img src="c2.png"></a><a href="/c3"><img src="c3.png"></a></div>
        </main>
    </body></html>"#;
    let content = extract_main_content(html);
    assert!(
        content.contains("Win with Velou"),
        "Should include hero text. Got: {content}"
    );
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
    assert_eq!(meta.og_image.as_deref(), Some("https://img.com/pic.jpg"));
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

// ── Listing-gate fixture tests (Phase 1) ──

mod listing_gate {
    use crw_extract::readability::{
        ProvenanceKind, ReadabilityOutcome, RejectReason, extract_main_content_with_provenance,
    };
    use std::fs;

    fn fixture(name: &str) -> String {
        fs::read_to_string(format!("tests/fixtures/listings/{name}"))
            .unwrap_or_else(|_| panic!("missing fixture: {name}"))
    }

    fn outcome_kind(o: &ReadabilityOutcome) -> &'static str {
        match o {
            ReadabilityOutcome::Selected { provenance, .. } => match provenance.kind {
                ProvenanceKind::Primary => "primary",
                ProvenanceKind::ListingFallback => "listing_fallback",
                ProvenanceKind::ListingRootRejected => "listing_root_rejected",
                ProvenanceKind::ReferenceProtected => "reference_protected",
            },
            ReadabilityOutcome::Rejected { reason } => match reason {
                RejectReason::ListingRootEmpty => "rejected_listing_root_empty",
                RejectReason::NoBodyAboveMinChars => "rejected_no_body",
            },
        }
    }

    /// Article fixtures must NOT be classified as listings.
    #[test]
    fn wikipedia_article_is_primary() {
        let o = extract_main_content_with_provenance(&fixture("wikipedia_article.html"));
        assert_eq!(outcome_kind(&o), "primary");
    }

    #[test]
    fn pmc_references_protected_as_primary() {
        // Reference-section heuristic must keep PMC bibliographies in
        // the primary path (recall would crater otherwise).
        let o = extract_main_content_with_provenance(&fixture("pmc_references.html"));
        assert_eq!(outcome_kind(&o), "primary");
    }

    #[test]
    fn forum_thread_is_primary() {
        let o = extract_main_content_with_provenance(&fixture("forum_thread.html"));
        assert_eq!(outcome_kind(&o), "primary");
    }

    #[test]
    fn roundup_with_paragraph_island_is_primary() {
        // Mixed page: long lead paragraph + card list. The paragraph-island
        // guard should keep it on the primary path.
        let o = extract_main_content_with_provenance(&fixture("roundup_with_island.html"));
        assert_eq!(outcome_kind(&o), "primary");
    }

    #[test]
    fn harada_data_table_is_primary() {
        let o = extract_main_content_with_provenance(&fixture("harada_table.html"));
        assert_eq!(outcome_kind(&o), "primary");
    }

    // Listing-trigger fixtures (card_grid, smartcompany_homepage, docs_toc)
    // removed: gate is now disabled (returns false unconditionally) so these
    // would always fail. See dom_util::is_listing_container for context.
}
