//! DOM-side features fed into the markdown quality scorer.
//!
//! Computed once per readability candidate. `text_density` is the
//! piecewise-saturating density used by `quality::analyze` to add a
//! "looks-like-an-article" bonus to candidates whose DOM has heavy text
//! relative to its tag count.

use scraper::ElementRef;

#[derive(Debug, Clone, Default)]
pub struct DomFeatures {
    /// Piecewise-normalized text density in `[0.0, 1.0]`. Saturates near
    /// 1.0 around raw density 125.
    pub text_density: f64,
    /// Anchor-to-text length ratio for the candidate root. Used by the
    /// dynamic primary-threshold path; not (yet) read by `quality.rs`.
    pub link_ratio: f64,
    /// Tag name of the chosen root, e.g. `"article"`, `"main"`, `"div"`.
    pub primary_root_tag: String,
}

/// Compute DOM features for a chosen readability container.
pub fn compute(root: ElementRef<'_>) -> DomFeatures {
    let total_text_chars: usize = root.text().map(|s| s.chars().count()).sum();
    let element_count = count_descendant_elements(root).max(1);
    let raw_density = total_text_chars as f64 / element_count as f64;
    let text_density = normalized_text_density(raw_density);

    let anchor_chars = anchor_text_chars(root);
    let link_ratio = if total_text_chars == 0 {
        0.0
    } else {
        (anchor_chars as f64 / total_text_chars as f64).clamp(0.0, 1.0)
    };

    DomFeatures {
        text_density,
        link_ratio,
        primary_root_tag: root.value().name().to_string(),
    }
}

/// Piecewise saturating mapping. Linear up to density 10, gentler in
/// `[10, 25]`, slow climb to ~1.0 at density 125. Matches the v6 plan
/// shape so dense layout-like roots (link farms) don't dominate the
/// quality bonus while real articles still hit the high band.
pub fn normalized_text_density(raw: f64) -> f64 {
    if raw <= 0.0 {
        0.0
    } else if raw <= 10.0 {
        raw / 25.0
    } else if raw <= 25.0 {
        0.4 + (raw - 10.0) / 75.0
    } else {
        0.6 + ((raw - 25.0) / 100.0).min(0.4)
    }
}

fn count_descendant_elements(el: ElementRef<'_>) -> usize {
    let mut n = 0;
    for desc in el.descendants() {
        if desc.value().is_element() {
            n += 1;
        }
    }
    n
}

fn anchor_text_chars(root: ElementRef<'_>) -> usize {
    let sel = scraper::Selector::parse("a").unwrap();
    root.select(&sel)
        .map(|a| a.text().map(|s| s.chars().count()).sum::<usize>())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use scraper::{Html, Selector};

    fn first_root(html: &str, sel: &str) -> DomFeatures {
        let doc = Html::parse_document(html);
        let s = Selector::parse(sel).unwrap();
        let root = doc.select(&s).next().expect("missing root");
        compute(root)
    }

    #[test]
    fn density_piecewise_monotonic() {
        let xs = [0.0, 5.0, 10.0, 15.0, 25.0, 50.0, 100.0, 125.0, 200.0];
        let ys: Vec<f64> = xs.iter().map(|x| normalized_text_density(*x)).collect();
        for w in ys.windows(2) {
            assert!(w[1] >= w[0], "non-monotonic at {w:?}");
        }
        assert!(ys.last().unwrap() <= &1.0);
    }

    #[test]
    fn density_caps_near_one() {
        assert!((normalized_text_density(125.0) - 1.0).abs() < 1e-9);
        assert!(normalized_text_density(1000.0) <= 1.0);
    }

    #[test]
    fn article_root_has_high_density_low_links() {
        let mut html = String::from("<!doctype html><html><body><article>");
        for _ in 0..6 {
            html.push_str("<p>Researchers studying migration patterns observed that flocks travel surprising distances each season. The findings suggest environmental cues guide navigation across continents.</p>");
        }
        html.push_str("</article></body></html>");
        let f = first_root(&html, "article");
        assert!(f.text_density > 0.6, "got {}", f.text_density);
        assert!(f.link_ratio < 0.1);
    }

    #[test]
    fn listing_root_has_low_density_high_links() {
        let mut html = String::from("<!doctype html><html><body><div class=\"cards\">");
        for i in 0..12 {
            html.push_str(&format!(
                "<article class=\"card\"><a href=\"/p/{i}\"><h3>Title {i}</h3></a></article>"
            ));
        }
        html.push_str("</div></body></html>");
        let f = first_root(&html, "div.cards");
        assert!(f.link_ratio > 0.5, "got {}", f.link_ratio);
        assert!(f.text_density < 0.5, "got {}", f.text_density);
    }
}
