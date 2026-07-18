use crw_core::types::ScrapedImage;
use once_cell::sync::Lazy;
use regex::Regex;
use scraper::{Html, Selector};
use std::collections::HashMap;

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
    /// Every other `<meta name|property>` tag on the page, keyed by its raw
    /// name/property (e.g. `twitter:creator`, `author`). Values are the `content`
    /// attribute; a tag that repeats becomes an array. Keys already surfaced as
    /// a named field above (`title`, `description`) are excluded to avoid a
    /// duplicate key once flattened onto the metadata object.
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
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

    let extra = collect_meta_tags(&document);

    ExtractedMetadata {
        title,
        description,
        language,
        og_title,
        og_description,
        og_image,
        canonical_url,
        extra,
    }
}

/// Collect every `<meta name|property>` tag into a map, mirroring Firecrawl's
/// flat metadata. `name` wins over `property` when both are present. A tag that
/// appears more than once (e.g. `viewport`) becomes a JSON array; a single tag
/// stays a string. `title` / `description` are skipped — they already ship as
/// named fields and would collide once flattened onto the metadata object.
fn collect_meta_tags(document: &Html) -> std::collections::BTreeMap<String, serde_json::Value> {
    use serde_json::Value;
    use std::collections::BTreeMap;

    const SKIP: [&str; 2] = ["title", "description"];
    let mut raw: BTreeMap<String, Vec<String>> = BTreeMap::new();

    if let Ok(sel) = Selector::parse("meta") {
        for el in document.select(&sel) {
            let attrs = el.value();
            let Some(key) = attrs.attr("name").or_else(|| attrs.attr("property")) else {
                continue;
            };
            let key = key.trim();
            let Some(content) = attrs.attr("content") else {
                continue;
            };
            let content = content.trim();
            if key.is_empty() || content.is_empty() || SKIP.contains(&key) {
                continue;
            }
            raw.entry(key.to_string())
                .or_default()
                .push(content.to_string());
        }
    }

    raw.into_iter()
        .map(|(k, mut vals)| {
            let v = if vals.len() == 1 {
                Value::String(vals.pop().unwrap())
            } else {
                Value::Array(vals.into_iter().map(Value::String).collect())
            };
            (k, v)
        })
        .collect()
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

/// `background-image: url(...)` extractor. Mirrors Firecrawl's `URL_REGEX`
/// (`apps/api/native/src/html.rs`) verbatim, including its `[^'")]+` stop — a
/// `)` inside a `data:` SVG can truncate the match. Kept for byte-for-byte
/// parity with the v2 drop-in surface.
// ponytail: naive `[^'")]+`; a CSS-value parser is the upgrade path if a real
// page needs it, but that would diverge the v2 URL set from Firecrawl.
static BG_URL_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"url\(['"]?([^'")]+)['"]?\)"#).unwrap());

/// Extract candidate URL tokens from an HTML `srcset` per the WHATWG parse
/// algorithm's URL step: each candidate's URL is a leading run of NON-whitespace
/// characters, so an internal comma in a `data:` URI stays part of the URL
/// rather than splitting it. After the URL, an optional descriptor runs to the
/// next top-level comma (parens tracked for `calc()` widths). For ordinary
/// `a.jpg 480w, b.jpg 1080w` srcsets this yields exactly `["a.jpg", "b.jpg"]`,
/// identical to a naive comma split; it only differs on comma-bearing URLs.
fn srcset_url_tokens(srcset: &str) -> Vec<&str> {
    fn is_ws(c: u8) -> bool {
        matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0c)
    }
    let b = srcset.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut urls = Vec::new();
    while i < n {
        while i < n && (is_ws(b[i]) || b[i] == b',') {
            i += 1;
        }
        if i >= n {
            break;
        }
        let start = i;
        while i < n && !is_ws(b[i]) {
            i += 1;
        }
        let mut url = &srcset[start..i];
        if url.ends_with(',') {
            // Trailing commas mean this candidate had no descriptor.
            url = url.trim_end_matches(',');
        } else {
            // Skip the descriptor up to the next top-level comma.
            let mut depth: i32 = 0;
            while i < n {
                match b[i] {
                    b'(' => depth += 1,
                    b')' => depth = depth.saturating_sub(1),
                    b',' if depth == 0 => break,
                    _ => {}
                }
                i += 1;
            }
        }
        if !url.is_empty() {
            urls.push(url);
        }
    }
    urls
}

/// Normalize an optional `alt`: trim and treat empty as absent.
fn norm_alt(alt: Option<&str>) -> Option<String> {
    alt.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Extract all images discovered on the page.
///
/// The **URL set mirrors Firecrawl's `_extract_images`** exactly (same sources,
/// resolution, filters, and srcset/background parsing — bugs included) so the v2
/// surface, which flattens these to a plain `Vec<String>`, stays a Firecrawl
/// drop-in from this single pass. The only native-`/v1` enrichment is the `alt`
/// field, which does not affect the URL set.
///
/// Deduplicated by URL in document order; a later duplicate carrying a non-empty
/// `alt` upgrades an earlier `None` (lazy-load and `<picture>`+`<img>` commonly
/// put the good `alt` on the second sighting).
pub fn extract_images(html: &str, base_url: &str) -> Vec<ScrapedImage> {
    let document = Html::parse_document(html);

    // Join base: honor `<base href>` when present, else the scrape source URL.
    // A `<base href>` may itself be relative (`/cdn/`) or absolute; resolve it
    // against the document URL (matching Firecrawl's `new URL(baseHref, baseUrl)`
    // fallback) so relative bases aren't silently dropped. A malformed base
    // degrades to `None` (absolute-only) rather than panicking.
    let doc_base = url::Url::parse(base_url).ok();
    // `<base href>` join base: relative or absolute, resolved against the doc URL;
    // `doc_base` itself is kept for protocol-relative (`//`) page-scheme joins.
    let base = select_attr(&document, "base[href]", "href")
        .and_then(|h| doc_base.as_ref().and_then(|b| b.join(&h).ok()))
        .or_else(|| doc_base.clone());

    // Resolve a raw src, mirroring Firecrawl's `resolve_image_url` branch-for-
    // branch so the v2 URL set stays a drop-in:
    //   data:/blob:    -> verbatim
    //   http(s):// abs -> verbatim (Firecrawl does NOT canonicalize absolutes)
    //   //host/x       -> inherit the PAGE scheme (join against the doc URL)
    //   relative       -> join against `<base href>` (falls back to the doc URL)
    // Then Firecrawl's final filter: drop `javascript:` (case-insensitive) and
    // any non-`data:`/`blob:` result that won't `Url::parse`.
    let resolve = |src: &str| -> Option<String> {
        // Deliberate, recall-neutral divergence from Firecrawl on degenerate
        // input: trim whitespace and skip an empty `src`. Firecrawl uses the raw
        // value, so its native resolver turns `src=""` into `base_href.join("")`
        // = the PAGE URL and emits it as an "image" (junk no drop-in client
        // wants), and keeps whitespace-padded URLs verbatim. We never drop a real
        // image here, so v2 recall is unaffected; we only omit that junk.
        let src = src.trim();
        if src.is_empty() {
            return None;
        }
        // Kept verbatim (Firecrawl does not canonicalize these). The
        // `http(s)://` prefix check is case-sensitive, exactly like Firecrawl —
        // an uppercase scheme (`HTTPS://`) intentionally falls through to
        // `join`, matching Firecrawl's `resolve_image_url`.
        let candidate = if src.starts_with("data:")
            || src.starts_with("blob:")
            || src.starts_with("http://")
            || src.starts_with("https://")
        {
            src.to_string()
        } else if src.starts_with("//") {
            // Protocol-relative: inherit the PAGE scheme (join against doc URL).
            doc_base.as_ref()?.join(src).ok()?.to_string()
        } else {
            base.as_ref()?.join(src).ok()?.to_string()
        };
        if candidate.to_ascii_lowercase().starts_with("javascript:") {
            return None;
        }
        if !candidate.starts_with("data:")
            && !candidate.starts_with("blob:")
            && url::Url::parse(&candidate).is_err()
        {
            return None;
        }
        Some(candidate)
    };

    // Dedup by URL in traversal order via a url->index map (O(1) per push, so a
    // page with many repeated URLs stays linear). Traversal order is the fixed
    // source-category order below (img, then picture, meta, icons, poster,
    // background) and DOM order within each — deterministic, matching Firecrawl.
    // A later duplicate carrying a non-empty `alt` upgrades an earlier `None`.
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut images: Vec<ScrapedImage> = Vec::new();
    let mut push = |url: String, alt: Option<String>| match index.get(&url) {
        None => {
            index.insert(url.clone(), images.len());
            images.push(ScrapedImage { url, alt });
        }
        Some(&i) => {
            if let Some(new_alt) = alt
                && images[i].alt.is_none()
            {
                images[i].alt = Some(new_alt);
            }
        }
    };

    // Extract candidate URLs from a `srcset`, resolved. Uses the WHATWG URL
    // step (`srcset_url_tokens`) rather than a naive `split(',')`: identical to
    // the naive result for ordinary `url 480w, url 1080w` srcsets, but a comma
    // INSIDE a `data:` URI no longer splits it into phantom fragments (a real
    // lazy-load placeholder pattern — see the smashingmagazine.com regression
    // test). Recall-neutral: it never drops a real image, only avoids junk.
    let srcset_urls = |srcset: &str| -> Vec<String> {
        srcset_url_tokens(srcset)
            .into_iter()
            .filter_map(&resolve)
            .collect()
    };

    // 1. <img src|data-src|srcset> — carries the img's alt.
    if let Ok(sel) = Selector::parse("img") {
        for el in document.select(&sel) {
            let alt = norm_alt(el.value().attr("alt"));
            if let Some(src) = el.value().attr("src")
                && let Some(url) = resolve(src)
            {
                push(url, alt.clone());
            }
            if let Some(src) = el.value().attr("data-src")
                && let Some(url) = resolve(src)
            {
                push(url, alt.clone());
            }
            if let Some(srcset) = el.value().attr("srcset") {
                for url in srcset_urls(srcset) {
                    push(url, alt.clone());
                }
            }
        }
    }

    // 2. <picture><source srcset> — no alt.
    if let Ok(sel) = Selector::parse("picture source") {
        for el in document.select(&sel) {
            if let Some(srcset) = el.value().attr("srcset") {
                for url in srcset_urls(srcset) {
                    push(url, None);
                }
            }
        }
    }

    // 3. OG / Twitter / itemprop meta images (read `content`) — no alt.
    if let Ok(sel) = Selector::parse(
        r#"meta[property="og:image"], meta[property="og:image:url"], meta[property="og:image:secure_url"], meta[name="twitter:image"], meta[name="twitter:image:src"], meta[itemprop="image"]"#,
    ) {
        for el in document.select(&sel) {
            if let Some(content) = el.value().attr("content")
                && let Some(url) = resolve(content)
            {
                push(url, None);
            }
        }
    }

    // 4. Icon / image_src links (read `href`, substring `*=` like Firecrawl) — no alt.
    if let Ok(sel) = Selector::parse(
        r#"link[rel*="icon"], link[rel*="apple-touch-icon"], link[rel*="image_src"]"#,
    ) {
        for el in document.select(&sel) {
            if let Some(href) = el.value().attr("href")
                && let Some(url) = resolve(href)
            {
                push(url, None);
            }
        }
    }

    // 5. <video poster> — no alt.
    if let Ok(sel) = Selector::parse("video[poster]") {
        for el in document.select(&sel) {
            if let Some(poster) = el.value().attr("poster")
                && let Some(url) = resolve(poster)
            {
                push(url, None);
            }
        }
    }

    // 6. Inline background-image styles — no alt.
    if let Ok(sel) = Selector::parse(r#"[style*="background"]"#) {
        for el in document.select(&sel) {
            if let Some(style) = el.value().attr("style") {
                for cap in BG_URL_REGEX.captures_iter(style) {
                    if let Some(m) = cap.get(1)
                        && let Some(url) = resolve(m.as_str())
                    {
                        push(url, None);
                    }
                }
            }
        }
    }

    images
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
    fn srcset_url_tokens_ordinary() {
        assert_eq!(
            srcset_url_tokens("a.jpg 480w, b.jpg 1080w, c.jpg 2x"),
            vec!["a.jpg", "b.jpg", "c.jpg"]
        );
    }

    #[test]
    fn srcset_url_tokens_no_descriptors() {
        // Trailing-comma candidates (no descriptor) still split correctly.
        assert_eq!(srcset_url_tokens("a.jpg, b.jpg"), vec!["a.jpg", "b.jpg"]);
    }

    #[test]
    fn srcset_url_tokens_data_uri_comma_kept() {
        // The comma inside the data: URI does NOT split the token.
        assert_eq!(
            srcset_url_tokens("data:image/avif;base64,AAAA== 1x, /real.jpg 2x"),
            vec!["data:image/avif;base64,AAAA==", "/real.jpg"]
        );
    }

    #[test]
    fn srcset_url_tokens_paren_descriptor() {
        // A descriptor containing a comma inside parens isn't a candidate split.
        assert_eq!(
            srcset_url_tokens("a.jpg 100w, b.jpg calc(50vw - 10px)"),
            vec!["a.jpg", "b.jpg"]
        );
    }

    #[test]
    fn srcset_url_tokens_empty() {
        assert!(srcset_url_tokens("").is_empty());
        assert!(srcset_url_tokens("   ").is_empty());
    }

    #[test]
    fn extracts_title_and_description() {
        let html = r#"<html><head><title>Test Page</title><meta name="description" content="A test"></head><body></body></html>"#;
        let meta = extract_metadata(html);
        assert_eq!(meta.title.unwrap(), "Test Page");
        assert_eq!(meta.description.unwrap(), "A test");
    }

    #[test]
    fn collects_arbitrary_meta_tags() {
        use serde_json::Value;
        let html = r#"<html><head>
            <title>T</title>
            <meta name="description" content="D">
            <meta name="twitter:creator" content="@behramcelen">
            <meta property="og:type" content="blog">
            <meta name="viewport" content="a">
            <meta name="viewport" content="b">
            <meta name="empty" content="">
        </head><body></body></html>"#;
        let meta = extract_metadata(html);
        // Arbitrary name/property tags surface verbatim.
        assert_eq!(
            meta.extra.get("twitter:creator"),
            Some(&Value::String("@behramcelen".into()))
        );
        assert_eq!(
            meta.extra.get("og:type"),
            Some(&Value::String("blog".into()))
        );
        // Repeated tag becomes an array.
        assert_eq!(
            meta.extra.get("viewport"),
            Some(&Value::Array(vec![
                Value::String("a".into()),
                Value::String("b".into())
            ]))
        );
        // title/description are named fields — excluded to avoid flatten collision.
        assert!(!meta.extra.contains_key("description"));
        // Empty content is dropped.
        assert!(!meta.extra.contains_key("empty"));
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
