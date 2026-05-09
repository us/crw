use scraper::{Html, Selector};

/// When a priority selector is "too broad" (>90% of body), drill down into it
/// to find a narrower content element.
fn find_content_within(parent_el: &scraper::ElementRef, parent_len: usize) -> Option<String> {
    let inner_selectors = [
        ".main-page-content",
        ".article-content",
        ".post-content",
        ".entry-content",
        ".content-body",
        ".article-body",
        "[itemprop=\"articleBody\"]",
        "[itemprop=\"text\"]",
        ".mw-parser-output",
        "#mw-content-text",
        "#content",
        ".content",
        "article", // nested article inside broad main
    ];

    let mut best: Option<(String, f64)> = None;
    for sel_str in &inner_selectors {
        if let Ok(sel) = Selector::parse(sel_str) {
            for el in parent_el.select(&sel) {
                let content = el.html();
                if content.len() < 200 {
                    continue;
                }
                // Skip if still too broad relative to parent
                if content.len() as f64 / parent_len as f64 > 0.85 {
                    continue;
                }
                let score = text_density(&content) * (content.len() as f64).ln();
                if best.as_ref().is_none_or(|(_, s)| score > *s) {
                    best = Some((content, score));
                }
            }
        }
    }
    best.map(|(c, _)| c)
}

/// Extract the "main content" element from HTML.
///
/// Uses text-density scoring across candidate selectors to pick the richest element.
/// Falls back to the `<body>` if no scored candidate is found.
pub fn extract_main_content(html: &str) -> String {
    let document = Html::parse_document(html);

    // Priority candidates in order: well-known semantic selectors first.
    let priority_selectors = ["article", "main", "[role=\"main\"]"];

    // Compute body length once for ratio checks below.
    let body_len = Selector::parse("body")
        .ok()
        .and_then(|sel| document.select(&sel).next())
        .map(|b| b.html().len())
        .unwrap_or(html.len());

    // Collect all candidates from priority selectors and score them.
    // Iterate in priority order so ties favor earlier selectors (article > main > role=main).
    let mut candidates: Vec<(scraper::ElementRef, String, f64, usize)> = Vec::new();
    for sel_str in &priority_selectors {
        if let Ok(sel) = Selector::parse(sel_str) {
            for el in document.select(&sel) {
                let content = el.html();
                if content.len() <= 200 {
                    continue;
                }
                let density = text_density(&content);
                if density <= 0.1 {
                    continue;
                }
                let text_len: usize = el.text().map(|t| t.len()).sum();
                if text_len == 0 {
                    continue;
                }
                let text_len_f = text_len as f64;

                let heading_count = ["h1", "h2", "h3", "h4", "h5", "h6"]
                    .iter()
                    .filter_map(|s| Selector::parse(s).ok())
                    .map(|s| el.select(&s).count())
                    .sum::<usize>();
                let paragraph_count = Selector::parse("p")
                    .ok()
                    .map(|s| el.select(&s).count())
                    .unwrap_or(0);
                let link_text_len: usize = Selector::parse("a")
                    .ok()
                    .map(|s| {
                        el.select(&s)
                            .map(|a| a.text().map(|t| t.len()).sum::<usize>())
                            .sum()
                    })
                    .unwrap_or(0);
                let link_density = link_text_len as f64 / text_len_f;

                let mut score = text_len_f * density
                    + (heading_count as f64) * 50.0
                    + (paragraph_count as f64) * 10.0
                    - link_density * text_len_f;

                // Penalty for filter/nav/sidebar markers in class or id.
                let attrs = format!(
                    "{} {}",
                    el.value().attr("class").unwrap_or(""),
                    el.value().attr("id").unwrap_or("")
                )
                .to_lowercase();
                const PENALTY_TOKENS: &[&str] =
                    &["filter", "facet", "sidebar", "nav", "menu", "navigation"];
                if PENALTY_TOKENS.iter().any(|t| attrs.contains(t)) {
                    score -= text_len_f * 0.7;
                }

                candidates.push((el, content, score, text_len));
            }
        }
    }

    if !candidates.is_empty() {
        // Find best by score; on tie, earlier (priority order) wins.
        let mut best_idx = 0;
        for i in 1..candidates.len() {
            if candidates[i].2 > candidates[best_idx].2 {
                best_idx = i;
            }
        }
        // Fallback guard: if best is much smaller than second-best by text length,
        // distrust and fall through to scored-selector path.
        let best_text_len = candidates[best_idx].3;
        let second_best_text_len = candidates
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != best_idx)
            .map(|(_, c)| c.3)
            .max()
            .unwrap_or(0);
        let trust_best = (second_best_text_len as f64) * 0.5 <= best_text_len as f64;

        if trust_best {
            let (el, content, _, _) = &candidates[best_idx];
            // If chosen element wraps nearly the entire document, drill down.
            if body_len > 0 && content.len() as f64 / body_len as f64 > 0.9 {
                if let Some(narrowed) = find_content_within(el, content.len()) {
                    return narrowed;
                }
                // Too broad and no narrower child — fall through to scoring.
            } else {
                return content.clone();
            }
        }
    }

    // Score all candidate selectors by text density and pick the best.
    let scored_selectors = [
        ".post-content",
        ".article-body",
        ".entry-content",
        ".article-content",
        ".post-body",
        ".story-body",
        ".content-body",
        "#main-content",
        "#article",
        "#content",
        ".content",
        ".main",
        "[itemprop=\"articleBody\"]",
        "[itemprop=\"text\"]",
        // MDN
        ".main-page-content",
        // StackOverflow
        ".js-post-body",
        ".s-prose",
        "#question",
        // Generic
        ".page-content",
        "#page-content",
        "[role=\"article\"]",
        // Wikipedia / MediaWiki
        ".mw-parser-output",
        "#mw-content-text",
        "#bodyContent",
        ".mw-body-content",
    ];

    let mut best: Option<(String, f64)> = None;
    for sel_str in &scored_selectors {
        if let Ok(sel) = Selector::parse(sel_str)
            && let Some(el) = document.select(&sel).next()
        {
            let content = el.html();
            if content.len() < 100 {
                continue;
            }
            // Skip selectors that wrap nearly the entire body (same as priority check).
            if body_len > 0 && content.len() as f64 / body_len as f64 > 0.9 {
                if let Some(narrowed) = find_content_within(&el, content.len()) {
                    return narrowed;
                }
                continue;
            }
            let score = text_density(&content) * (content.len() as f64).ln();
            if best.as_ref().is_none_or(|(_, s)| score > *s) {
                best = Some((content, score));
            }
        }
    }

    if let Some((content, _)) = best {
        return content;
    }

    // Last resort: return full body.
    if let Ok(sel) = Selector::parse("body")
        && let Some(body) = document.select(&sel).next()
    {
        return body.inner_html();
    }

    html.to_string()
}

/// Provenance of a successful main-content extraction.
#[derive(Debug, Clone)]
pub struct Provenance {
    pub kind: ProvenanceKind,
    pub candidate_features: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvenanceKind {
    /// Standard text-density / priority-selector pick.
    Primary,
    /// Picked element was a listing root; we detached repeating subtrees
    /// or descended into a non-listing child.
    ListingFallback,
    /// Listing detected but no usable body recovered.
    ListingRootRejected,
    /// Element lives inside a reference / bibliography section.
    ReferenceProtected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectReason {
    /// Picked element was a listing and detach/descent both failed.
    ListingRootEmpty,
    /// No candidate cleared the minimum-body threshold.
    NoBodyAboveMinChars,
}

#[derive(Debug, Clone)]
pub enum ReadabilityOutcome {
    Selected {
        html: String,
        provenance: Provenance,
    },
    Rejected {
        reason: RejectReason,
    },
}

const LISTING_FALLBACK_MIN_CHARS: usize = 400;
const MAX_DESCENT_DEPTH: u8 = 3;

/// Provenance-aware variant of [`extract_main_content`].
///
/// First runs the legacy density-scored picker. If the chosen container
/// looks like a listing (cards, link grid), tries:
///
/// 1. detach repeating-shape subtrees in place;
/// 2. descend up to `MAX_DESCENT_DEPTH` looking for the largest
///    non-listing child with `>= LISTING_FALLBACK_MIN_CHARS` of text.
///
/// Returns `Rejected` only when both fallbacks fail to surface a body —
/// callers can then jump to the cleaned-HTML candidate.
pub fn extract_main_content_with_provenance(html: &str) -> ReadabilityOutcome {
    let primary = extract_main_content(html);
    if primary.trim().is_empty() {
        return ReadabilityOutcome::Rejected {
            reason: RejectReason::NoBodyAboveMinChars,
        };
    }

    let frag = Html::parse_fragment(&primary);
    let root_text_len = crate::dom_util::text_char_len(frag.root_element());
    // Readability often returns a wrapper (body/main/article) that has the
    // listing nested one or more levels deep. Walk the entire fragment tree
    // and trigger on the first descendant that matches the listing gate
    // — but only if it covers a meaningful share of the picked content
    // (≥50% of root text). Otherwise we'd treat sidebars / "more from"
    // rails as the page's primary intent.
    let listing_target = {
        let root = frag.root_element();
        find_listing_descendant(root).filter(|el| {
            if root_text_len == 0 {
                return false;
            }
            let target_text_len = crate::dom_util::text_char_len(*el);
            (target_text_len as f64) / (root_text_len as f64) >= 0.5
        })
    };
    if let Some(el) = listing_target {
        // Case B (listing root): try to descend into a non-listing child
        // with enough prose to stand on its own; otherwise reject and let
        // the caller fall through to the cleaned-HTML alternate, which
        // preserves card titles for downstream markdown conversion.
        if let Some(narrower) = walk_to_non_listing_descendant(el, MAX_DESCENT_DEPTH) {
            return ReadabilityOutcome::Selected {
                html: narrower,
                provenance: Provenance {
                    kind: ProvenanceKind::ListingFallback,
                    candidate_features: None,
                },
            };
        }
        return ReadabilityOutcome::Rejected {
            reason: RejectReason::ListingRootEmpty,
        };
    }

    ReadabilityOutcome::Selected {
        html: primary,
        provenance: Provenance {
            kind: ProvenanceKind::Primary,
            candidate_features: None,
        },
    }
}

fn find_listing_descendant<'a>(el: scraper::ElementRef<'a>) -> Option<scraper::ElementRef<'a>> {
    use crate::dom_util::{ElementChildren, has_paragraph_island, is_listing_container};
    // If any ancestor along the path has a paragraph island, the listing
    // is incidental (article with embedded card row) — leave it alone.
    if has_paragraph_island(el, LISTING_FALLBACK_MIN_CHARS) {
        return None;
    }
    if is_listing_container(el) {
        return Some(el);
    }
    for child in el.element_children() {
        if let Some(found) = find_listing_descendant(child) {
            return Some(found);
        }
    }
    None
}

fn walk_to_non_listing_descendant(el: scraper::ElementRef<'_>, max_depth: u8) -> Option<String> {
    use crate::dom_util::{ElementChildren, is_listing_container, text_char_len};
    if max_depth == 0 {
        return None;
    }
    let mut best: Option<(String, usize)> = None;
    for child in el.element_children() {
        if is_listing_container(child) {
            continue;
        }
        let chars = text_char_len(child);
        if chars < LISTING_FALLBACK_MIN_CHARS {
            continue;
        }
        let html = child.html();
        if best.as_ref().is_none_or(|(_, c)| chars > *c) {
            best = Some((html, chars));
        }
    }
    if let Some((h, _)) = best {
        return Some(h);
    }
    for child in el.element_children() {
        if let Some(v) = walk_to_non_listing_descendant(child, max_depth - 1) {
            return Some(v);
        }
    }
    None
}

/// Compute text-to-html ratio as a simple content density signal.
/// Returns a value in [0, 1]: higher = more text relative to markup.
fn text_density(html: &str) -> f64 {
    let doc = Html::parse_fragment(html);
    let text_len: usize = doc.root_element().text().map(|t| t.len()).sum();
    if html.is_empty() {
        return 0.0;
    }
    text_len as f64 / html.len() as f64
}

/// All extracted metadata from a page.
pub struct ExtractedMetadata {
    pub title: Option<String>,
    pub description: Option<String>,
    pub language: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub og_image: Option<String>,
    pub canonical_url: Option<String>,
}

/// Extract metadata (title, description, OG tags, canonical) from HTML.
pub fn extract_metadata(html: &str) -> ExtractedMetadata {
    let document = Html::parse_document(html);

    let title = select_text(&document, "title");

    let description = select_attr(&document, r#"meta[name="description"]"#, "content");

    let og_title = select_attr(&document, r#"meta[property="og:title"]"#, "content");
    let og_description = select_attr(&document, r#"meta[property="og:description"]"#, "content");
    let og_image = select_attr(&document, r#"meta[property="og:image"]"#, "content");

    let canonical_url = select_attr(&document, r#"link[rel="canonical"]"#, "href");

    // Extract language from <html lang="..."> attribute.
    let language = select_attr(&document, "html", "lang");

    ExtractedMetadata {
        title,
        description,
        language,
        og_title,
        og_description,
        og_image,
        canonical_url,
    }
}

fn select_text(doc: &Html, selector: &str) -> Option<String> {
    Selector::parse(selector)
        .ok()
        .and_then(|sel| doc.select(&sel).next())
        .map(|el| el.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty())
}

fn select_attr(doc: &Html, selector: &str, attr: &str) -> Option<String> {
    Selector::parse(selector)
        .ok()
        .and_then(|sel| doc.select(&sel).next())
        .and_then(|el| el.value().attr(attr).map(|s| s.to_string()))
        .filter(|s| !s.is_empty())
}

/// Extract all links from HTML.
pub fn extract_links(html: &str, base_url: &str) -> Vec<String> {
    let document = Html::parse_document(html);
    let sel = match Selector::parse("a[href]") {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    let base = url::Url::parse(base_url).ok();

    document
        .select(&sel)
        .filter_map(|el| {
            let href = el.value().attr("href")?;
            if href.starts_with('#')
                || href.starts_with("javascript:")
                || href.starts_with("mailto:")
                || href.starts_with("data:")
                || href.starts_with("tel:")
                || href.starts_with("blob:")
            {
                return None;
            }
            if let Some(base) = &base {
                base.join(href).ok().map(|u| u.to_string())
            } else if href.starts_with("http") {
                Some(href.to_string())
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_article_content() {
        let html = r#"<html><body><nav>Nav</nav><article><p>Main content</p></article><footer>Foot</footer></body></html>"#;
        let content = extract_main_content(html);
        assert!(content.contains("Main content"));
    }

    #[test]
    fn extracts_title_and_description() {
        let html = r#"<html><head><title>Test Page</title><meta name="description" content="A test"></head><body></body></html>"#;
        let meta = extract_metadata(html);
        assert_eq!(meta.title.unwrap(), "Test Page");
        assert_eq!(meta.description.unwrap(), "A test");
    }

    #[test]
    fn extracts_og_metadata() {
        let html = r#"<html><head>
            <meta property="og:title" content="OG Title">
            <meta property="og:description" content="OG Desc">
            <meta property="og:image" content="https://img.com/pic.jpg">
            <link rel="canonical" href="https://example.com/canonical">
        </head><body></body></html>"#;
        let meta = extract_metadata(html);
        assert_eq!(meta.og_title.unwrap(), "OG Title");
        assert_eq!(meta.og_description.unwrap(), "OG Desc");
        assert_eq!(meta.og_image.unwrap(), "https://img.com/pic.jpg");
        assert_eq!(meta.canonical_url.unwrap(), "https://example.com/canonical");
    }

    #[test]
    fn skips_broad_article_picks_mw_parser_output() {
        // Simulate Wikipedia structure: <article> wraps everything,
        // but .mw-parser-output is the real content.
        let filler = "x".repeat(500);
        let html = format!(
            r#"<html><body>
            <article>
              <div id="mw-navigation">{filler}</div>
              <div id="content" role="main">
                <div id="bodyContent">
                  <div id="mw-content-text">
                    <div class="mw-parser-output">
                      <p>This is the real Wikipedia article content about web scraping. {filler}</p>
                    </div>
                  </div>
                </div>
              </div>
              <div class="catlinks">{filler}</div>
            </article>
            </body></html>"#
        );
        let content = extract_main_content(&html);
        assert!(
            content.contains("real Wikipedia article content"),
            "Should extract .mw-parser-output content"
        );
        // Should NOT contain the navigation or catlinks filler
        assert!(
            !content.contains("mw-navigation"),
            "Should not include navigation div"
        );
    }

    #[test]
    fn extracts_links() {
        let html = r##"<html><body><a href="/page1">P1</a><a href="https://other.com">O</a><a href="#top">T</a></body></html>"##;
        let links = extract_links(html, "https://example.com");
        assert_eq!(links.len(), 2);
        assert!(links.contains(&"https://example.com/page1".to_string()));
        assert!(
            links.contains(&"https://other.com".to_string())
                || links.contains(&"https://other.com/".to_string())
        );
    }
}
