use crw_extract::readability::{
    extract_images, extract_links, extract_main_content, extract_metadata,
};

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

// ── Image Extraction ──

/// Collect just the URLs, in order.
fn img_urls(imgs: &[crw_core::types::ScrapedImage]) -> Vec<String> {
    imgs.iter().map(|i| i.url.clone()).collect()
}

#[test]
fn extract_images_img_src_and_alt() {
    let html = r#"<html><body><img src="/pic.png" alt="A cat"></body></html>"#;
    let imgs = extract_images(html, "https://example.com/dir/");
    assert_eq!(imgs.len(), 1);
    assert_eq!(imgs[0].url, "https://example.com/pic.png");
    assert_eq!(imgs[0].alt.as_deref(), Some("A cat"));
}

#[test]
fn extract_images_relative_and_protocol_relative_resolved() {
    let html = r#"<html><body>
        <img src="a.png"><img src="/b.png"><img src="//cdn.example.net/c.png">
        </body></html>"#;
    let urls = img_urls(&extract_images(html, "https://example.com/dir/"));
    assert!(urls.contains(&"https://example.com/dir/a.png".to_string()));
    assert!(urls.contains(&"https://example.com/b.png".to_string()));
    assert!(urls.contains(&"https://cdn.example.net/c.png".to_string()));
}

#[test]
fn extract_images_keeps_data_and_blob_drops_javascript() {
    let html = r#"<html><body>
        <img src="data:image/png;base64,AAAA">
        <img src="blob:https://example.com/xyz">
        <img src="javascript:alert(1)">
        <img src="JavaScript:alert(2)">
        </body></html>"#;
    let urls = img_urls(&extract_images(html, "https://example.com"));
    assert!(urls.contains(&"data:image/png;base64,AAAA".to_string()));
    assert!(urls.iter().any(|u| u.starts_with("blob:")));
    // both case variants of javascript: are dropped
    assert!(
        !urls
            .iter()
            .any(|u| u.to_lowercase().contains("javascript:"))
    );
}

#[test]
fn extract_images_srcset_first_token() {
    let html = r#"<html><body>
        <img srcset="/small.png 480w, /large.png 1080w" alt="responsive">
        </body></html>"#;
    let urls = img_urls(&extract_images(html, "https://example.com"));
    assert!(urls.contains(&"https://example.com/small.png".to_string()));
    assert!(urls.contains(&"https://example.com/large.png".to_string()));
}

#[test]
fn extract_images_srcset_data_uri_not_split_into_phantoms() {
    // Regression (smashingmagazine.com): a `data:` URI in srcset contains a
    // comma; the WHATWG URL parse keeps it whole instead of splitting it into
    // a truncated `data:...;base64` + a phantom `<base>/AAAA...` path.
    let html = r#"<html><body><picture>
        <source srcset="data:image/avif;base64,AAAABBBBCCCC== 1x, /real.jpg 2x">
        </picture></body></html>"#;
    let urls = img_urls(&extract_images(html, "https://example.com/page"));
    assert!(
        urls.contains(&"data:image/avif;base64,AAAABBBBCCCC==".to_string()),
        "data URI must be kept whole: {urls:?}"
    );
    assert!(urls.contains(&"https://example.com/real.jpg".to_string()));
    // No phantom URL built from the base64 payload.
    assert!(
        !urls
            .iter()
            .any(|u| u.contains("AAAABBBBCCCC") && u.starts_with("http")),
        "no phantom base64-as-path URL: {urls:?}"
    );
}

#[test]
fn extract_images_srcset_ordinary_matches_naive() {
    // Ordinary descriptor srcset yields exactly the URL tokens (parity with the
    // old naive split, and with Firecrawl).
    let html = r#"<html><body><img srcset="/a.jpg 480w, /b.jpg 1080w, /c.jpg 2x"></body></html>"#;
    let urls = img_urls(&extract_images(html, "https://example.com"));
    for e in [
        "https://example.com/a.jpg",
        "https://example.com/b.jpg",
        "https://example.com/c.jpg",
    ] {
        assert!(urls.contains(&e.to_string()), "missing {e}: {urls:?}");
    }
}

#[test]
fn extract_images_picture_source_no_alt() {
    let html = r#"<html><body><picture>
        <source srcset="/hero.webp 1x">
        <img src="/hero.png" alt="hero">
        </picture></body></html>"#;
    let imgs = extract_images(html, "https://example.com");
    let webp = imgs.iter().find(|i| i.url.ends_with("hero.webp")).unwrap();
    assert_eq!(webp.alt, None);
}

#[test]
fn extract_images_meta_and_icon_and_poster_and_background() {
    let html = r#"<html><head>
        <meta property="og:image" content="/og.png">
        <meta name="twitter:image" content="https://example.com/tw.png">
        <link rel="mask-icon" href="/mask.svg">
        <link rel="apple-touch-icon" href="/touch.png">
        </head><body>
        <video poster="/poster.jpg"></video>
        <div style="background-image: url('/bg.png')"></div>
        </body></html>"#;
    let urls = img_urls(&extract_images(html, "https://example.com"));
    for expected in [
        "https://example.com/og.png",
        "https://example.com/tw.png",
        "https://example.com/mask.svg", // rel*="icon" must catch mask-icon
        "https://example.com/touch.png",
        "https://example.com/poster.jpg",
        "https://example.com/bg.png",
    ] {
        assert!(urls.contains(&expected.to_string()), "missing {expected}");
    }
}

#[test]
fn extract_images_dedup_preserves_order_and_upgrades_alt() {
    // Same URL first with no alt (lazy placeholder), then with a real alt.
    let html = r#"<html><body>
        <img src="/x.png">
        <img src="/y.png" alt="Y">
        <img src="/x.png" alt="Real X">
        </body></html>"#;
    let imgs = extract_images(html, "https://example.com");
    let urls = img_urls(&imgs);
    // x appears once, keeping its first-seen position (before y)
    assert_eq!(
        urls,
        vec![
            "https://example.com/x.png".to_string(),
            "https://example.com/y.png".to_string(),
        ]
    );
    // and its alt was upgraded from None to the later non-empty one
    assert_eq!(imgs[0].alt.as_deref(), Some("Real X"));
}

#[test]
fn extract_images_empty_alt_is_none() {
    let html = r#"<html><body><img src="/p.png" alt="   "></body></html>"#;
    let imgs = extract_images(html, "https://example.com");
    assert_eq!(imgs[0].alt, None);
}

#[test]
fn extract_images_none_found_is_empty() {
    let html = r#"<html><body><p>no images here</p></body></html>"#;
    assert!(extract_images(html, "https://example.com").is_empty());
}

#[test]
fn extract_images_empty_src_skipped_not_page_url() {
    // An empty/whitespace src must NOT resolve to the page URL (Firecrawl's
    // native resolver would emit `base` here; we skip it as junk).
    let html = r#"<html><body><img src=""><img src="   "><img src="/real.png"></body></html>"#;
    let urls = img_urls(&extract_images(html, "https://example.com/page"));
    assert_eq!(urls, vec!["https://example.com/real.png".to_string()]);
}

#[test]
fn extract_images_honors_base_href() {
    let html = r#"<html><head><base href="https://cdn.example.net/assets/"></head>
        <body><img src="pic.png"></body></html>"#;
    let urls = img_urls(&extract_images(html, "https://example.com/page"));
    assert!(urls.contains(&"https://cdn.example.net/assets/pic.png".to_string()));
}

#[test]
fn extract_images_honors_relative_base_href() {
    // A relative <base href> resolves against the document URL (not dropped).
    let html = r#"<html><head><base href="/cdn/"></head>
        <body><img src="pic.png"></body></html>"#;
    let urls = img_urls(&extract_images(html, "https://example.com/a/b"));
    assert!(
        urls.contains(&"https://example.com/cdn/pic.png".to_string()),
        "relative base href must resolve against the doc URL: {urls:?}"
    );
}

#[test]
fn extract_images_absolute_url_returned_verbatim() {
    // Firecrawl parity: absolute http(s) URLs are NOT canonicalized (host case,
    // explicit port, etc. preserved) so v2 strings match exactly. Uses a
    // lowercase scheme (Firecrawl's prefix check, like ours, is case-sensitive).
    let html = r#"<html><body><img src="https://CDN.Example.COM:8443/Logo.PNG"></body></html>"#;
    let urls = img_urls(&extract_images(html, "https://example.com"));
    assert!(urls.contains(&"https://CDN.Example.COM:8443/Logo.PNG".to_string()));
}

#[test]
fn extract_images_protocol_relative_uses_page_scheme() {
    // `//host/x` inherits the PAGE scheme, even when <base href> has a different
    // scheme (matches Firecrawl, which joins protocol-relative against the page).
    let html = r#"<html><head><base href="http://other.example/"></head>
        <body><img src="//cdn.example.net/p.png"></body></html>"#;
    let urls = img_urls(&extract_images(html, "https://example.com/page"));
    assert!(
        urls.contains(&"https://cdn.example.net/p.png".to_string()),
        "protocol-relative must inherit the page (https) scheme: {urls:?}"
    );
}
