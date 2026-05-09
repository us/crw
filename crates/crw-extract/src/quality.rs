//! Markdown quality scoring used to drive escalation logic.
//!
//! Discriminates between real article content and degenerate output such as
//! image-only pages or boilerplate filter sidebars.

use crate::dom_features::DomFeatures;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy)]
pub struct Quality {
    pub bytes: usize,
    pub words: usize,
    pub unique_words: usize,
    pub avg_line_len: f32,
    pub link_or_image_ratio: f32,
    pub boilerplate_ratio: f32,
    pub score: f32,
}

const BOILERPLATE_TOKENS: &[&str] = &[
    "sidebar",
    "filter",
    "facet",
    " nav ",
    " menu ",
    "cookie",
    "consent",
    "subscribe",
    "newsletter",
    "accept all",
    "manage preferences",
    "privacy policy",
    "terms of service",
    "all rights reserved",
    "our services",
    "meet the team",
    "find a doctor",
    "find a professional",
    "pay my bill",
    "contact us",
    "about us",
    "recent posts",
    "search for",
    "©",
];

/// Markdown-only convenience wrapper for callers without DOM context
/// (plaintext path, llm comparator, structured extraction). Forwards
/// to [`analyze`] with `dom = None`.
pub fn analyze_md_only(markdown: &str) -> Quality {
    analyze(markdown, None)
}

pub fn analyze(markdown: &str, dom: Option<&DomFeatures>) -> Quality {
    let bytes = markdown.len();

    // Tokenize: alphanumeric or apostrophe, len >= 2.
    let mut words: usize = 0;
    let mut uniq: HashSet<String> = HashSet::new();
    for raw in markdown.split_ascii_whitespace() {
        let tok: String = raw
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '\'')
            .collect();
        if tok.len() >= 2 {
            words += 1;
            uniq.insert(tok.to_lowercase());
        }
    }
    let unique_words = uniq.len();

    // Non-empty lines.
    let lines: Vec<&str> = markdown.lines().filter(|l| !l.trim().is_empty()).collect();
    let avg_line_len = if lines.is_empty() {
        0.0
    } else {
        let total: usize = lines.iter().map(|l| l.chars().count()).sum();
        total as f32 / lines.len() as f32
    };

    // Markdown link/image proxy.
    let link_or_image_count = markdown.matches("](").count();
    let link_or_image_ratio = (link_or_image_count as f32 / words.max(1) as f32).min(1.0);

    // Boilerplate detection (line-level, lowercased; surround with spaces so
    // ` nav ` / ` menu ` patterns match at line boundaries too).
    let boilerplate_lines = lines
        .iter()
        .filter(|l| {
            let lc = format!(" {} ", l.to_lowercase());
            BOILERPLATE_TOKENS.iter().any(|t| lc.contains(t))
        })
        .count();
    let boilerplate_ratio = if lines.is_empty() {
        0.0
    } else {
        boilerplate_lines as f32 / lines.len() as f32
    };

    let unique_ratio = unique_words as f32 / words.max(1) as f32;
    // Weight recalibration (v6 plan, mirrors crawl4ai composite):
    // OLD: + 0.5 * (1 - link_or_image_ratio)
    // NEW: piecewise link_penalty applied at weight 0.2; plus a DOM
    //      text-density bonus (0.0 when dom is unavailable).
    let r = link_or_image_ratio.min(1.0);
    let link_penalty = if r < 0.3 {
        0.0
    } else {
        let t = (r - 0.3) / 0.7;
        t * t
    };
    let dom_density_bonus = dom.map(|d| 0.4 * d.text_density as f32).unwrap_or(0.0);
    let mut score = (words.min(800) as f32 / 800.0) + dom_density_bonus
        - 0.2 * link_penalty
        - 1.0 * boilerplate_ratio
        - 0.3 * (1.0 - unique_ratio);
    score = score.clamp(-1.0, 2.0);

    Quality {
        bytes,
        words,
        unique_words,
        avg_line_len,
        link_or_image_ratio,
        boilerplate_ratio,
        score,
    }
}

pub fn is_low_quality(q: &Quality) -> bool {
    q.score < 0.4 && q.words < 200
}

#[cfg(test)]
mod tests {
    use super::*;

    fn high_quality_markdown() -> String {
        // Diverse-vocabulary article so unique_ratio doesn't sink the
        // recalibrated score (v6: identical-paragraph corpora penalised
        // by 0.3 * (1 - unique_ratio); the prior fixture repeated a
        // single template ten times and now scores low for legitimate
        // reasons).
        let mut s = String::new();
        s.push_str("# Migratory Patterns Across Continents\n\n");
        s.push_str("## Introduction\n\nOrnithologists tracking arctic terns recorded the longest annual migration ever observed, spanning more than seventy thousand kilometres between polar feeding grounds.\n\n");
        s.push_str("## Methodology\n\nGeolocator devices weighing under a gram captured solar elevation data at five-minute intervals, allowing post-recovery reconstruction of complete flight trajectories.\n\n");
        s.push_str("## Field Observations\n\nResearchers documented opportunistic stopovers along previously unmapped oceanic ridges where upwelling currents concentrate krill and small forage fish populations.\n\n");
        s.push_str("## Climate Signal\n\nWarming sea-surface temperatures appear to shift staging-area arrival dates earlier by roughly two days per decade, decoupling traditional alignment with prey availability windows.\n\n");
        s.push_str("## Genetic Markers\n\nNuclear and mitochondrial sequencing revealed cryptic divergence between western and eastern populations despite apparent overlap on shared wintering grounds south of the equator.\n\n");
        s.push_str("## Acoustic Signatures\n\nAutomated recording units distinguished individual contact calls from neighbouring colonies, enabling fine-grained estimates of philopatry and dispersal among breeding cohorts.\n\n");
        s.push_str("## Predator Pressure\n\nGreat skuas and arctic foxes exerted measurable influence on nest-site selection, with successful pairs preferring elevated micro-habitats sheltered from prevailing summer winds.\n\n");
        s.push_str("## Conservation Outlook\n\nOngoing fisheries reform and protected-area expansion offer the most plausible levers for stabilising long-distance migrants whose routes intersect multiple regulatory jurisdictions.\n\n");
        s.push_str("## Tracking Innovations\n\nMiniaturised satellite transmitters now sample atmospheric pressure, ambient temperature and wing-beat cadence, producing rich behavioural inferences alongside positional fixes once thought sufficient on their own.\n\n");
        s.push_str("## Modelling Uncertainty\n\nHierarchical Bayesian frameworks accommodate variable detection probability across heterogeneous landscapes, sharply tightening parameter estimates compared with earlier maximum-likelihood approaches that treated absences naively.\n\n");
        s.push_str("See the [original paper](https://example.com/paper) for full statistical appendices and supplementary tables.\n");
        s
    }

    #[test]
    fn high_quality_article() {
        // Markdown-only path: with the v6 weight recalibration the
        // baseline `+0.5` link-ratio bonus is gone, so a moderate-length
        // article without a DOM signal lands in the mid-band rather than
        // automatically clearing 0.6. The is_low_quality gate
        // (`score < 0.4 AND words < 200`) is the load-bearing check.
        let q = analyze_md_only(&high_quality_markdown());
        assert!(!is_low_quality(&q), "should not be flagged low: {q:?}");

        // With the DOM-density bonus (production path), the same content
        // clears the historical 0.6 bar — verifying the recalibrated
        // composite still ranks real articles strongly.
        let dom = DomFeatures {
            text_density: 1.0,
            link_ratio: 0.05,
            primary_root_tag: "article".into(),
        };
        let q_dom = analyze(&high_quality_markdown(), Some(&dom));
        assert!(
            q_dom.score > 0.6,
            "with DOM bonus expected > 0.6, got {}",
            q_dom.score
        );
    }

    #[test]
    fn image_only_low_quality() {
        let md = "![](a.jpg)\n![](b.jpg)\n![](c.jpg)\n![](d.jpg)\n![](e.jpg)\n\
                  ![](f.jpg)\n![](g.jpg)\n![](h.jpg)\n![](i.jpg)\n![](j.jpg)\n";
        let q = analyze_md_only(md);
        assert!(q.score < 0.3, "expected score < 0.3, got {}", q.score);
        assert!(q.words < 200);
        assert!(is_low_quality(&q));
    }

    #[test]
    fn boilerplate_heavy_filter() {
        let mut s = String::new();
        for _ in 0..6 {
            s.push_str("Sort by relevance\n");
            s.push_str("Distance: 25 miles\n");
            s.push_str("Job Type: Full time filter\n");
            s.push_str("Sidebar facet panel\n");
            s.push_str("Filter results here\n");
        }
        let q = analyze_md_only(&s);
        assert!(
            q.boilerplate_ratio > 0.3,
            "expected boilerplate_ratio > 0.3, got {}",
            q.boilerplate_ratio
        );
        // Score should be depressed by the boilerplate penalty.
        let baseline = (q.words.min(800) as f32 / 800.0) + 0.5;
        assert!(q.score < baseline);
    }

    #[test]
    fn score_ordering() {
        let high = analyze_md_only(&high_quality_markdown());
        let low = analyze_md_only("![](a.jpg)\n![](b.jpg)\n![](c.jpg)\n");
        assert!(high.score > low.score);
    }
}
