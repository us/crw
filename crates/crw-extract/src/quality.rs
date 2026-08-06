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
    /// Share of the text that sits in bare navigation entries: short lines that
    /// are almost entirely one link. Language-independent, unlike
    /// [`BOILERPLATE_TOKENS`].
    ///
    /// Measured in words, not lines. A docs page carries a long sidebar of
    /// one-word links beside a real article; by line count the nav dominates and
    /// the page looks like pure chrome, by word count it is the minority it
    /// actually is.
    pub chrome_ratio: f32,
    pub score: f32,
}

/// Weight of the nav-line penalty in the composite score.
///
/// Zero on purpose. Penalising nav here demotes the one candidate that carried
/// the article body on docs and help-centre pages, where a long sidebar sits
/// beside real prose, and the ladder then falls through to a thin candidate
/// that lost the body (measured: 63 real phrases dropped across 25 pages).
/// Nav is removed from the winner instead, by [`strip_nav_lines`], which keeps
/// the body and drops the menu. The field is still computed because that
/// removal and its content floor are both driven by it.
const CHROME_WEIGHT: f32 = 0.0;

/// Longest visible line still treated as a possible nav entry. Menu labels are
/// short; a link-heavy line longer than this is usually a real sentence that
/// happens to cite sources.
const CHROME_MAX_VISIBLE: usize = 80;

/// Most words a nav entry carries. This is what separates a menu label from a
/// linked headline, and it is the whole reason the rule is safe to apply: an
/// author page, a blog index and a docs index are all lists of links whose text
/// is a real title, and those titles run long. "Dow Jones Newswires" is three
/// words; "Introducing HoneyBee: How We Automate Honeypot Deployment for Threat
/// Research" is ten. Without this bound the rule ate 119 real phrases across 58
/// pages of the frozen set. Counted with [`label_tokens`] so a spaceless script
/// is measured in characters rather than reported as a single word.
const CHROME_MAX_WORDS: usize = 4;

/// A nav entry looks the same in every language: a short line, usually a list
/// item, that is almost entirely one link. `[Barron's](https://barrons.com)`
/// and `[Ana Sayfa](/)` are the same shape, and neither is caught by the
/// English phrase list below, which scores 0 on 68% of the article corpus.
///
/// Reads the line once, tracking how many visible characters sit inside link
/// text versus outside it.
fn is_link_chrome(line: &str) -> bool {
    let mut s = line.trim();
    // Strip one markdown list marker: "* ", "- ", "+ ", "1. ".
    if let Some(rest) = s
        .strip_prefix('*')
        .or_else(|| s.strip_prefix('-'))
        .or_else(|| s.strip_prefix('+'))
    {
        if rest.starts_with(char::is_whitespace) {
            s = rest.trim_start();
        }
    } else {
        let digits = s.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && s[digits..].starts_with(". ") {
            s = s[digits + 2..].trim_start();
        }
    }

    // Walk the line once, collecting what a reader would see. URLs are not
    // visible text: counting them made `[会社概要](/company)` look like a
    // five-word line.
    //
    // The obvious optimisation, counting tokens inline to skip this buffer, was
    // tried and measured slower (21.8s vs 20.2s over the 948-page set), so the
    // straightforward version stays.
    let (mut in_link, mut outside, mut links) = (0usize, 0usize, 0usize);
    let mut visible_text = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            // `[text](url)` — keep `text`, skip `(url)` entirely.
            if let Some(close) = s[i..].find("](") {
                let text = &s[i + 1..i + close];
                if let Some(paren) = s[i + close + 2..].find(')') {
                    in_link += text.chars().filter(|c| !c.is_whitespace()).count();
                    visible_text.push_str(text);
                    visible_text.push(' ');
                    links += 1;
                    i += close + 2 + paren + 1;
                    continue;
                }
            }
        }
        let ch = s[i..].chars().next().unwrap_or(' ');
        if !ch.is_whitespace() {
            outside += 1;
        }
        visible_text.push(ch);
        i += ch.len_utf8();
    }

    let visible = in_link + outside;
    links > 0
        && visible > 0
        && visible <= CHROME_MAX_VISIBLE
        && in_link * 5 >= visible * 4
        && label_tokens(&visible_text) <= CHROME_MAX_WORDS
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

/// How many words must survive the strip for it to be worth doing.
///
/// This is the guard that keeps a gallery, a docs index or a product listing
/// intact: there the links ARE the content, and removing them leaves nothing.
/// It deliberately does NOT look at the share of the text that is nav. A
/// paywalled news page is mostly menu by volume and still has an article under
/// it, and a share-based floor protected exactly those worst cases.
const CHROME_MIN_BODY_WORDS: usize = 60;

/// A menu arrives as a block. One link line on its own is far more likely to be
/// content: a byline of linked author names, a contact address, a single
/// reference. Requiring a run keeps those.
const CHROME_MIN_RUN: usize = 4;

/// Drop bare navigation lines from finished markdown.
///
/// Three guards keep this from eating content: enough text has to survive
/// ([`CHROME_MIN_BODY_WORDS`]), the lines have to arrive as a block
/// ([`CHROME_MIN_RUN`]), and each has to be short enough to be a label rather
/// than a headline ([`CHROME_MAX_WORDS`]).
///
/// Returns `None` when nothing was removed, letting callers avoid a copy.
pub fn strip_nav_lines(markdown: &str) -> Option<String> {
    // Classify and count in one pass. Calling `analyze` here instead cost
    // ~3.5ms per page on the frozen set, nearly all of it building the
    // unique-word set, which this does not need.
    let lines: Vec<&str> = markdown.lines().collect();
    let mut chrome = vec![false; lines.len()];
    let (mut body_words, mut chrome_words) = (0usize, 0usize);
    for (i, line) in lines.iter().enumerate() {
        let w = count_words(line);
        if is_link_chrome(line) {
            chrome[i] = true;
            chrome_words += w;
        } else {
            body_words += w;
        }
    }
    if chrome_words == 0 || body_words < CHROME_MIN_BODY_WORDS {
        return None;
    }

    // Keep only the nav lines that sit in a run.
    let mut drop = vec![false; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        if !chrome[i] {
            i += 1;
            continue;
        }
        // A menu block is often split by blank lines, one group per submenu.
        // Blanks continue the run without counting toward its length; any real
        // line ends it.
        let (mut j, mut last, mut count) = (i, i, 0usize);
        while j < lines.len() {
            if chrome[j] {
                count += 1;
                last = j;
            } else if !lines[j].trim().is_empty() {
                break;
            }
            j += 1;
        }
        if count >= CHROME_MIN_RUN {
            drop[i..=last].fill(true);
        }
        i = j.max(i + 1);
    }
    if !drop.iter().any(|d| *d) {
        return None;
    }

    let mut out = String::with_capacity(markdown.len());
    let mut blank_run = 0usize;
    for (idx, line) in lines.iter().enumerate() {
        if drop[idx] {
            continue;
        }
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    Some(out.trim_end().to_string())
}

/// Same tokenization the composite score uses, applied to one line.
fn count_words(line: &str) -> usize {
    line.split_ascii_whitespace()
        .filter(|raw| {
            raw.chars()
                .filter(|c| c.is_alphanumeric() || *c == '\'')
                .count()
                >= 2
        })
        .count()
}

/// Scripts that do not put spaces between words. Counting whitespace tokens in
/// Japanese returns 1 for a whole sentence, which made a Japanese table of
/// contents look like a four-word menu label.
fn is_spaceless_script(c: char) -> bool {
    matches!(c,
        '\u{3040}'..='\u{30FF}'   // hiragana + katakana
        | '\u{3400}'..='\u{4DBF}' // CJK ext A
        | '\u{4E00}'..='\u{9FFF}' // CJK unified
        | '\u{F900}'..='\u{FAFF}' // CJK compatibility
        | '\u{AC00}'..='\u{D7AF}' // hangul syllables
        | '\u{0E00}'..='\u{0E7F}' // thai
    )
}

/// Length of a candidate menu label, in units comparable across scripts: a
/// whitespace-separated word, or a single character of a spaceless script.
fn label_tokens(s: &str) -> usize {
    let spaceless = s.chars().filter(|c| is_spaceless_script(*c)).count();
    let spaced = s
        .split_ascii_whitespace()
        .filter(|w| {
            w.chars()
                .any(|c| c.is_alphanumeric() && !is_spaceless_script(c))
        })
        .count();
    spaceless + spaced
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

    let chrome_words: usize = lines
        .iter()
        .filter(|l| is_link_chrome(l))
        .map(|l| count_words(l))
        .sum();
    let chrome_ratio = if words == 0 {
        0.0
    } else {
        (chrome_words as f32 / words as f32).min(1.0)
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
        - CHROME_WEIGHT * chrome_ratio
        - 0.3 * (1.0 - unique_ratio);
    score = score.clamp(-1.0, 2.0);

    Quality {
        bytes,
        words,
        unique_words,
        avg_line_len,
        link_or_image_ratio,
        boilerplate_ratio,
        chrome_ratio,
        score,
    }
}

pub fn is_low_quality(q: &Quality) -> bool {
    q.score < 0.4 && q.words < 200
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nav_run_goes_article_stays() {
        let body: String = (0..12)
            .map(|i| format!("Paragraph {i} carries several words of ordinary prose text.\n\n"))
            .collect();
        let md = format!(
            "# Headline\n\n\
             *   [Barron's](https://www.barrons.com)\n\
             *   [BigCharts](http://bigcharts.marketwatch.com)\n\
             *   [Factiva](https://global.factiva.com/login)\n\
             *   [Financial News](https://www.fnlondon.com/)\n\n\
             {body}"
        );
        let out = strip_nav_lines(&md).expect("a four-link menu is a run");
        assert!(!out.contains("BigCharts"), "menu survived:\n{out}");
        assert!(out.contains("Paragraph 0"), "body was eaten:\n{out}");
        assert!(out.contains("# Headline"));
    }

    #[test]
    fn linked_headlines_and_lone_links_survive() {
        // An author page: every line is a link, but each is a real title. Also
        // covers the run rule, since two of these sit together.
        let md = "# Author\n\n\
             *   [Introducing HoneyBee: How We Automate Honeypot Deployment for Research](/a)\n\
             *   [Turning attacker insights into stronger cloud security protections](/b)\n\
             *   [What the latest cloud threat data says about identity attacks](/c)\n\n"
            .to_string()
            + &(0..12)
                .map(|i| format!("Paragraph {i} carries several words of ordinary prose text.\n\n"))
                .collect::<String>();
        assert!(
            strip_nav_lines(&md).is_none_or(|o| o.contains("HoneyBee")),
            "linked headlines must not read as a menu"
        );
    }

    #[test]
    fn spaceless_script_is_not_a_menu_label() {
        // Counted as whitespace words this Japanese heading is one "word" and
        // looks like a menu entry; counted per character it plainly is not.
        assert!(!is_link_chrome(
            "*   [1.1 ライブコマースを始める企業がなぜ増えているのか](/x)"
        ));
        assert!(is_link_chrome("*   [会社概要](/company)"));
    }

    #[test]
    fn a_listing_page_keeps_its_links() {
        let md: String = (0..30)
            .map(|i| format!("*   [Photo {i}](/p/{i})\n"))
            .collect();
        assert!(
            strip_nav_lines(&md).is_none(),
            "with no body left, the links are the page"
        );
    }

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
