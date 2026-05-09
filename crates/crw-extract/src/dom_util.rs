//! Lightweight DOM walking helpers shared by the listing-detection gate.
//!
//! Centralised here so `readability.rs`, `tables.rs`, and `dom_features.rs`
//! all use the same notion of "element child", "anchor text length", and
//! "structural signature".

use ego_tree::NodeId;
use scraper::ElementRef;
use std::collections::HashMap;
use xxhash_rust::xxh3::xxh3_64;

/// Iterator over an element's *element* children (skipping text /
/// comment nodes). `ElementRef` is `Copy`, so the call consumes a copy
/// of the binding rather than the original.
pub trait ElementChildren<'a> {
    fn element_children(self) -> Box<dyn Iterator<Item = ElementRef<'a>> + 'a>;
}

impl<'a> ElementChildren<'a> for ElementRef<'a> {
    fn element_children(self) -> Box<dyn Iterator<Item = ElementRef<'a>> + 'a> {
        Box::new(self.children().filter_map(ElementRef::wrap))
    }
}

/// Total grapheme-count of the element's text descendants. We count
/// chars (Rust's Unicode scalar values) as a cheap stand-in for true
/// graphemes — close enough for ratio comparisons.
pub fn text_char_len(el: ElementRef<'_>) -> usize {
    el.text().map(|s| s.chars().count()).sum()
}

/// Total chars contained inside `<a>` descendants of this element.
pub fn link_char_len(el: ElementRef<'_>) -> usize {
    let sel = scraper::Selector::parse("a").unwrap();
    el.select(&sel)
        .map(|a| a.text().map(|s| s.chars().count()).sum::<usize>())
        .sum()
}

/// `(text - link) / text` clamped to `[0, 1]`. 1.0 means no anchor
/// content; 0.0 means everything is inside `<a>`.
pub fn non_link_char_ratio(el: ElementRef<'_>) -> f64 {
    let total = text_char_len(el);
    if total == 0 {
        return 1.0;
    }
    let link = link_char_len(el);
    let non = total.saturating_sub(link);
    (non as f64 / total as f64).clamp(0.0, 1.0)
}

/// Recursive structural signature of `el` to depth `depth`. Skips
/// elements whose visible text is entirely whitespace (firecrawl
/// `html.rs:483` parity). Memoised by `NodeId` across a single page.
pub fn node_signature(el: ElementRef<'_>, depth: u8, memo: &mut HashMap<NodeId, u64>) -> u64 {
    let id = el.id();
    if let Some(cached) = memo.get(&id) {
        return *cached;
    }
    // Empty-text guard.
    let txt: String = el.text().collect();
    if txt.trim().is_empty() {
        memo.insert(id, 0);
        return 0;
    }

    let kids: Vec<&str> = el.element_children().map(|c| c.value().name()).collect();
    let bucket: u8 = match kids.len() {
        0 => 0,
        1 => 1,
        2..=4 => 2,
        5..=9 => 3,
        _ => 4,
    };

    let mut buf = String::with_capacity(64 + kids.len() * 8);
    buf.push_str(el.value().name());
    buf.push('|');
    for k in &kids {
        buf.push_str(k);
        buf.push(',');
    }
    buf.push('|');
    buf.push((b'0' + bucket) as char);
    let mut sig = xxh3_64(buf.as_bytes());

    if depth > 0 {
        for (idx, child) in el.element_children().enumerate() {
            let rot = (((idx as u64).wrapping_mul(11)) ^ 7) % 64;
            let csig = node_signature(child, depth - 1, memo);
            sig ^= csig.rotate_left(rot as u32);
        }
    }

    memo.insert(id, sig);
    sig
}

/// Largest count of children sharing one structural signature.
pub fn count_repeating_shape_siblings(el: ElementRef<'_>) -> usize {
    let mut memo: HashMap<NodeId, u64> = HashMap::new();
    let mut groups: HashMap<u64, usize> = HashMap::new();
    for child in el.element_children() {
        let sig = node_signature(child, 2, &mut memo);
        if sig == 0 {
            continue;
        }
        *groups.entry(sig).or_insert(0) += 1;
    }
    groups.values().copied().max().unwrap_or(0)
}

/// Direct-child `<p>` elements with `>= min_chars` of non-link text.
pub fn has_paragraph_island(el: ElementRef<'_>, min_chars: usize) -> bool {
    el.element_children()
        .filter(|c| c.value().name().eq_ignore_ascii_case("p"))
        .any(|p| {
            let total = text_char_len(p);
            let link = link_char_len(p);
            total.saturating_sub(link) >= min_chars
        })
}

/// Walk ancestors looking for reference-section markers. Matches
/// common English / CJK / Slavic / German / French headings and
/// `role="doc-bibliography"` / class hooks.
pub fn in_reference_section(el: ElementRef<'_>) -> bool {
    let mut cur = el.parent();
    while let Some(node) = cur {
        if let Some(parent_el) = ElementRef::wrap(node) {
            let v = parent_el.value();
            if let Some(role) = v.attr("role")
                && role.eq_ignore_ascii_case("doc-bibliography")
            {
                return true;
            }
            let class = v.attr("class").unwrap_or("");
            let id = v.attr("id").unwrap_or("");
            let blob = format!("{class} {id}").to_ascii_lowercase();
            if blob.contains("references")
                || blob.contains("ref-list")
                || blob.contains("citation-list")
                || blob.contains("bibliography")
            {
                return true;
            }
            // Section heading sniff: first heading text inside <section>.
            if v.name() == "section"
                && let Some(h) = parent_el
                    .select(&scraper::Selector::parse("h1,h2,h3,h4").unwrap())
                    .next()
            {
                let head: String = h.text().collect::<String>().trim().to_lowercase();
                const KW: &[&str] = &[
                    "references",
                    "reference",
                    "bibliography",
                    "works cited",
                    "citations",
                    "further reading",
                    "参考文献",
                    "引用文献",
                    "참고문헌",
                    "참고 자료",
                    "литература",
                    "библиография",
                    "literatur",
                    "literaturverzeichnis",
                    "références",
                    "bibliographie",
                    "参考资料",
                ];
                if KW.iter().any(|k| head.starts_with(k) || head == *k) {
                    return true;
                }
            }
        }
        cur = node.parent();
    }
    false
}

/// Listing-container detector — kept as a stub.
///
/// Empirical 150-URL bench (Tier 6.5b) showed every threshold tuning
/// produced net negative truth-found flips: 7 lifts traded for 11
/// regressions, no_md count tripled. The gate adds more harm than help on
/// our dataset. Returns `false` unconditionally; readability runs
/// unmodified. The `lib.rs` extract() reactive size-guard handles short
/// markdown via the candidate ladder, which subsumed the gate's intent.
pub fn is_listing_container(_el: ElementRef<'_>) -> bool {
    false
}

/// Re-serialize `outer_html` of a candidate root with the children that
/// match the listing's repeating signature removed. Implementation
/// detail: identify the dominant signature group, collect each matching
/// child's `outer_html`, and replace each occurrence in the parent's
/// serialized HTML with an empty string. Returns `None` when there's
/// nothing to detach (no dominant group).
pub fn detach_repeating_subtrees(container: ElementRef<'_>) -> Option<String> {
    let mut memo: HashMap<NodeId, u64> = HashMap::new();
    let mut groups: HashMap<u64, Vec<ElementRef<'_>>> = HashMap::new();
    for child in container.element_children() {
        let sig = node_signature(child, 2, &mut memo);
        if sig == 0 {
            continue;
        }
        groups.entry(sig).or_default().push(child);
    }
    let dominant = groups.into_values().max_by_key(|v| v.len())?;
    if dominant.len() < 5 {
        return None;
    }
    let mut html = container.html();
    for el in dominant {
        let snippet = el.html();
        // Replace first occurrence; multiple identical subtrees mean
        // multiple replacements are still well-defined.
        html = html.replacen(&snippet, "", 1);
    }
    Some(html)
}

#[cfg(test)]
mod tests {
    use super::*;
    use scraper::{Html, Selector};

    fn root(html: &str, sel: &str) -> Html {
        let _ = sel;
        Html::parse_document(html)
    }

    #[test]
    fn paragraph_island_detected_above_threshold() {
        let html = format!(
            "<!doctype html><html><body><div><p>{}</p></div></body></html>",
            "long ".repeat(120)
        );
        let doc = root(&html, "div");
        let div = doc.select(&Selector::parse("div").unwrap()).next().unwrap();
        assert!(has_paragraph_island(div, 400));
    }

    #[test]
    fn paragraph_island_skipped_when_short() {
        let html = "<!doctype html><html><body><div><p>hi</p></div></body></html>";
        let doc = root(html, "div");
        let div = doc.select(&Selector::parse("div").unwrap()).next().unwrap();
        assert!(!has_paragraph_island(div, 400));
    }

    #[test]
    fn detach_repeating_returns_shorter_html() {
        let mut html = String::from("<!doctype html><html><body><section>");
        for i in 0..10 {
            html.push_str(&format!(
                "<article class=\"card\"><a href=\"/p/{i}\"><h3>Title {i} of the day</h3></a></article>"
            ));
        }
        html.push_str("</section></body></html>");
        let doc = Html::parse_document(&html);
        let sec = doc
            .select(&Selector::parse("section").unwrap())
            .next()
            .unwrap();
        let detached = detach_repeating_subtrees(sec).expect("should detach");
        assert!(detached.len() < sec.html().len());
        assert!(!detached.contains("Title 5"));
    }
}
