//! HTML content extraction and format conversion for the CRW web scraper.
//!
//! Converts raw HTML into clean, structured output formats:
//!
//! - **Markdown** — via [`markdown::html_to_markdown`] (htmd)
//! - **Plain text** — via [`plaintext::html_to_plaintext`]
//! - **Cleaned HTML** — boilerplate removal with [`clean::clean_html`]
//! - **Readability** — main-content extraction with text-density scoring
//! - **CSS/XPath selector** — narrow content to a specific element
//! - **Chunking** — split content into sentence/topic/regex chunks
//! - **Filtering** — BM25 or cosine-similarity ranking of chunks
//! - **Structured JSON** — LLM-based extraction with JSON Schema validation

pub mod antibot;
pub mod chunking;
pub mod clean;
pub mod dom_features;
pub mod dom_util;
pub mod filter;
pub mod judge;
pub mod markdown;
pub mod pdf;
pub mod plaintext;
pub mod quality;
pub mod readability;
pub mod selector;
pub mod structured;
pub mod tables;

use crw_core::error::{CrwError, CrwResult};
use crw_core::types::{
    CapturedNetworkResponse, ChunkResult, ChunkStrategy, DebugAttempt, DebugCandidate,
    DebugExtraction, FilterMode, OutputFormat, PageMetadata, RenderDecision, ScrapeData,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Per-request collector for extraction debug traces. Wired in through
/// [`ExtractOptions::debug_sink`]; the extractor pushes one
/// [`DebugAttempt`] per `extract()` invocation, capturing the candidate
/// ladder and the chosen output. Wrapped in an `Arc<Mutex<_>>` so the
/// renderer / multi-attempt loop in `crw-crawl` can share a single sink
/// across the JS-escalation retry.
#[derive(Debug, Default)]
pub struct DebugCollector {
    attempts: Vec<DebugAttempt>,
}

impl DebugCollector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_attempt(&mut self, attempt: DebugAttempt) {
        self.attempts.push(attempt);
    }

    pub fn into_extraction(self) -> DebugExtraction {
        DebugExtraction {
            attempts: self.attempts,
        }
    }
}

/// Convenience: lift a single candidate description into a
/// [`DebugCandidate`].
pub fn debug_candidate(
    kind: impl Into<String>,
    text: Option<String>,
    score: f64,
    cap_chars: Option<usize>,
) -> DebugCandidate {
    let text_excerpt = text.as_ref().map(|s| {
        let mut idx = 200.min(s.len());
        while idx > 0 && !s.is_char_boundary(idx) {
            idx -= 1;
        }
        s[..idx].to_string()
    });
    DebugCandidate {
        kind: kind.into(),
        text,
        text_excerpt,
        cap_chars,
        score,
    }
}

pub mod answer;
pub mod llm;
pub mod pricing;
pub mod summary;

/// Options for the high-level extraction pipeline.
pub struct ExtractOptions<'a> {
    pub raw_html: &'a str,
    pub source_url: &'a str,
    pub status_code: u16,
    pub rendered_with: Option<String>,
    pub elapsed_ms: u64,
    /// Routing decision metadata to surface to API consumers.
    pub render_decision: Option<RenderDecision>,
    /// Credit cost attributed to this fetch.
    pub credit_cost: u32,
    /// Soft-failure warnings collected through the render chain.
    pub warnings: Vec<String>,
    pub formats: &'a [OutputFormat],
    pub only_main_content: bool,
    pub include_tags: &'a [String],
    pub exclude_tags: &'a [String],
    /// CSS selector to narrow content before readability extraction.
    pub css_selector: Option<&'a str>,
    /// XPath expression to narrow content before readability extraction.
    pub xpath: Option<&'a str>,
    /// Strategy for chunking the extracted markdown.
    pub chunk_strategy: Option<&'a ChunkStrategy>,
    /// Query for chunk filtering (requires filter_mode).
    pub query: Option<&'a str>,
    /// Filtering algorithm for chunk ranking.
    pub filter_mode: Option<&'a FilterMode>,
    /// Number of top chunks to return (default: 5).
    pub top_k: Option<usize>,
    /// Per-host CSS selector overrides. Used only when the request did not
    /// supply an explicit `css_selector` / `xpath`. The selector for the
    /// source URL's host is applied before readability narrowing.
    pub domain_selectors: Option<&'a HashMap<String, String>>,
    /// XHR/fetch responses captured during navigation. Used as a fallback
    /// content source when DOM-based extraction is low quality.
    pub captured_responses: &'a [CapturedNetworkResponse],
    /// LLM-assisted extraction fallback configuration. When the chosen
    /// candidate's quality score is below `quality_threshold` and `enable`
    /// is true, the raw HTML (truncated to `max_html_bytes`) is sent to the
    /// configured LLM provider for re-extraction.
    pub llm_fallback: Option<LlmFallbackParams<'a>>,
    /// Opt-in extraction debug trace. When true, the extractor populates
    /// `debug_sink` with one [`DebugAttempt`] per `extract()` invocation.
    pub debug: bool,
    /// Sink for debug attempts. Shared across the multi-attempt
    /// JS-escalation loop so that all attempts land in one trace.
    pub debug_sink: Option<Arc<Mutex<DebugCollector>>>,
}

/// Parameters for the LLM-assisted extraction fallback. See
/// [`LlmFallbackConfig`](crw_core::config::LlmFallbackConfig).
#[derive(Debug, Clone)]
pub struct LlmFallbackParams<'a> {
    pub api_key: &'a str,
    pub model: &'a str,
    pub provider: &'a str,
    pub base_url: Option<&'a str>,
    pub quality_threshold: f32,
    pub max_html_bytes: usize,
    pub max_tokens: u32,
    pub azure_api_version: Option<&'a str>,
    /// When true, run the LLM regardless of DOM-extraction quality
    /// ("primary extractor" mode); when false, only run as a fallback for
    /// candidates scoring below `quality_threshold`.
    pub always_run: bool,
}

/// Re-extract via the configured LLM provider when the current markdown
/// scores below `params.quality_threshold`. If the LLM result has a higher
/// quality score, it replaces `data.markdown` in place and a warning is
/// appended noting the swap. On any failure (network, auth, parse) the
/// original markdown is preserved and the error is logged.
pub async fn maybe_run_llm_fallback(
    data: &mut ScrapeData,
    raw_html: &str,
    params: &LlmFallbackParams<'_>,
) -> CrwResult<()> {
    let current_md = match data.markdown.as_deref() {
        Some(m) if !m.trim().is_empty() => m,
        _ => "",
    };
    let current_quality = quality::analyze_md_only(current_md);
    if !params.always_run && current_quality.score >= params.quality_threshold {
        return Ok(());
    }
    match llm::extract_via_llm(
        raw_html,
        params.api_key,
        params.provider,
        params.model,
        params.base_url,
        params.max_tokens,
        params.max_html_bytes,
        params.azure_api_version,
    )
    .await
    {
        Ok(llm_md) => {
            let llm_quality = quality::analyze_md_only(&llm_md);
            if llm_quality.score > current_quality.score {
                tracing::info!(
                    prior_score = current_quality.score,
                    llm_score = llm_quality.score,
                    "LLM fallback produced higher-quality markdown"
                );
                data.markdown = Some(llm_md);
                data.warnings.push("extracted_via=llm".to_string());
            } else {
                tracing::debug!(
                    prior_score = current_quality.score,
                    llm_score = llm_quality.score,
                    "LLM fallback produced lower-quality markdown; keeping original"
                );
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "LLM fallback call failed; keeping DOM extraction");
        }
    }
    Ok(())
}

/// Look up the host-specific CSS selector override for a URL.
fn lookup_domain_selector(source_url: &str, map: &HashMap<String, String>) -> Option<String> {
    if map.is_empty() {
        return None;
    }
    let host = url::Url::parse(source_url)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_string()))?;
    map.get(&host).cloned()
}

#[cfg(test)]
mod private_tests {
    use super::*;
    use crw_core::types::CapturedNetworkResponse;

    #[test]
    fn domain_selector_matches_exact_host() {
        let mut map = HashMap::new();
        map.insert("news.example.com".to_string(), ".article".to_string());
        let got = lookup_domain_selector("https://news.example.com/p/42", &map);
        assert_eq!(got.as_deref(), Some(".article"));
    }

    #[test]
    fn domain_selector_misses_on_other_host() {
        let mut map = HashMap::new();
        map.insert("news.example.com".to_string(), ".article".to_string());
        let got = lookup_domain_selector("https://other.example.com/p/42", &map);
        assert!(got.is_none());
    }

    #[test]
    fn domain_selector_empty_map_returns_none() {
        let map = HashMap::new();
        assert!(lookup_domain_selector("https://x.example.com/", &map).is_none());
    }

    #[test]
    fn xhr_extract_returns_none_for_empty_input() {
        assert!(extract_xhr_text(&[]).is_none());
    }

    #[test]
    fn xhr_extract_collects_long_string_fields() {
        let body = serde_json::json!({
            "title": "short",
            "body": "a".repeat(300),
            "meta": { "summary": "b".repeat(200) },
            "tags": ["c".repeat(150), "short"],
            "url": "https://example.com/should/skip",
        })
        .to_string();
        let resp = vec![CapturedNetworkResponse {
            url: "https://api.example.com/article/1".to_string(),
            request_id: "1".to_string(),
            status: 200,
            mime_type: Some("application/json".to_string()),
            body: Some(body),
            body_size_bytes: 800,
        }];
        let got = extract_xhr_text(&resp).expect("expected long-text fields");
        assert!(got.contains(&"a".repeat(300)));
        assert!(got.contains(&"b".repeat(200)));
        assert!(got.contains(&"c".repeat(150)));
        assert!(!got.contains("short"));
        assert!(!got.contains("example.com/should/skip"));
    }

    #[test]
    fn xhr_extract_skips_invalid_json() {
        let resp = vec![CapturedNetworkResponse {
            url: "x".into(),
            request_id: "1".into(),
            status: 200,
            mime_type: Some("application/json".into()),
            body: Some("not json".into()),
            body_size_bytes: 8,
        }];
        assert!(extract_xhr_text(&resp).is_none());
    }
}

/// Decode the small set of HTML entities that commonly appear in `<meta>`
/// `content` attributes. We don't pull in a full entity decoder because the
/// metadata path only sees author-curated short text, and the long tail of
/// `&amp_lt_named_;` references is empty in practice.
fn decode_basic_html_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.char_indices();
    while let Some((i, ch)) = chars.next() {
        if ch != '&' {
            out.push(ch);
            continue;
        }
        let rest = &s[i..];
        let replacement: Option<(&str, &str)> = [
            ("&amp;", "&"),
            ("&lt;", "<"),
            ("&gt;", ">"),
            ("&quot;", "\""),
            ("&apos;", "'"),
            ("&#39;", "'"),
            ("&nbsp;", " "),
            ("&hellip;", "…"),
            ("&mdash;", "—"),
            ("&ndash;", "–"),
            ("&rsquo;", "\u{2019}"),
            ("&lsquo;", "\u{2018}"),
            ("&rdquo;", "\u{201D}"),
            ("&ldquo;", "\u{201C}"),
        ]
        .into_iter()
        .find(|(needle, _)| rest.starts_with(needle));
        if let Some((needle, value)) = replacement {
            out.push_str(value);
            for _ in 0..(needle.len() - 1) {
                chars.next();
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Collapse blank-line splits inside an inline punctuation-separated link list.
///
/// Pages like `dnb.com/business-directory/...` emit each `<a>` for an
/// industry tag inside its own block-level wrapper, so htmd serialises
/// `Industry:\u{a0}<a>X</a>, <a>Y</a>, Z` as
/// `Industry:\u{a0}\n\n[X],\n\n[Y],\n\nZ`. Substring matching against
/// `"Industry: X, Y, Z"` then fails over the embedded blank lines. The
/// rendered page (and our plainText output) keeps the items inline; only
/// the markdown emitter breaks them up. We undo that locally without
/// changing real paragraph structure: NBSP → space (markdown has no
/// NBSP semantics), then collapse exactly the blank-lines that sit
/// between trailing punctuation (`,`, `:`, `)`) and the next inline link
/// (`[`) or a continuing list-item word.
fn reflow_inline_lists(s: String) -> String {
    if !s.contains('\u{00a0}') && !s.contains(",\n\n") && !s.contains(":\n\n") {
        return s;
    }
    let mut t = s.replace('\u{00a0}', " ");
    // ":<spaces>?\n+<spaces>?[" → ": ["
    t = INLINE_LINK_AFTER_PUNCT.replace_all(&t, "$p [").into_owned();
    // "),<spaces>?\n+<spaces>?[" → "), ["
    t = INLINE_LINK_AFTER_CLOSE.replace_all(&t, "), [").into_owned();
    // ",<spaces>?\n+<spaces>?<letter>" → ", <letter>" (trailing list item that
    // isn't itself a link, e.g. "[X], [Y], \n\nMarketing consulting services")
    t = TRAILING_LIST_ITEM.replace_all(&t, ", $w").into_owned();
    t
}

static INLINE_LINK_AFTER_PUNCT: once_cell::sync::Lazy<regex::Regex> =
    once_cell::sync::Lazy::new(|| {
        regex::Regex::new(r"(?P<p>[,:])[ \t]*\n[\s]*\[").expect("inline-link regex compiles")
    });
static INLINE_LINK_AFTER_CLOSE: once_cell::sync::Lazy<regex::Regex> =
    once_cell::sync::Lazy::new(|| {
        regex::Regex::new(r"\),[ \t]*\n[\s]*\[").expect("inline-link close regex compiles")
    });
static TRAILING_LIST_ITEM: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| {
    regex::Regex::new(r",[ \t]*\n\n+(?P<w>[A-Za-z\u{00C0}-\u{FFFF}])")
        .expect("trailing list-item regex compiles")
});

/// High-level extraction: given raw HTML + options, produce ScrapeData.
pub fn extract(opts: ExtractOptions<'_>) -> CrwResult<ScrapeData> {
    let ExtractOptions {
        raw_html,
        source_url,
        status_code,
        rendered_with,
        elapsed_ms,
        render_decision,
        credit_cost,
        warnings,
        formats,
        only_main_content,
        include_tags,
        exclude_tags,
        css_selector,
        xpath,
        chunk_strategy,
        query,
        filter_mode,
        top_k,
        domain_selectors,
        captured_responses,
        llm_fallback: _,
        debug: _,
        debug_sink: _,
    } = opts;

    // Per-host fallback selector — used only when the caller didn't pass an
    // explicit css_selector / xpath. User input always wins over host defaults.
    // Track whether the *caller* opted into a narrow extraction; downstream
    // metadata injection (title prepend) keys off this, not the merged value,
    // so a domain-config default doesn't suppress the title fallback.
    let user_selected = css_selector.is_some() || xpath.is_some();
    let domain_selector_owned: Option<String> =
        if !user_selected && let Some(map) = domain_selectors {
            lookup_domain_selector(source_url, map)
        } else {
            None
        };
    let css_selector = css_selector.or(domain_selector_owned.as_deref());

    // Step 1: Extract metadata from raw HTML.
    let meta = readability::extract_metadata(raw_html);

    // Step 2: Clean HTML (remove boilerplate, nav, ads, etc.).
    let cleaned = clean::clean_html(raw_html, only_main_content, include_tags, exclude_tags)
        .unwrap_or_else(|_| raw_html.to_string());

    // Step 3: Apply CSS/XPath selector if provided (narrows to a specific element).
    let selected_html = apply_selector(&cleaned, css_selector, xpath)?;
    let after_selection = selected_html.as_deref().unwrap_or(&cleaned);

    // Step 4: If only_main_content, try to narrow further with readability scoring.
    let (content_html, cleaned_ref) = if only_main_content && selected_html.is_none() {
        match readability::extract_main_content_with_provenance(after_selection) {
            readability::ReadabilityOutcome::Selected { html: main, .. } => {
                // Re-clean: readability may have selected a broad container
                // (e.g. <article>) that still contains noise elements
                // (infobox, navbox, catlinks, etc.).
                let re_cleaned = clean::clean_html(&main, true, &[], &[]).unwrap_or(main);
                (re_cleaned, Some(cleaned))
            }
            readability::ReadabilityOutcome::Rejected { .. } => {
                // Listing root or empty body — skip readability and let the
                // alternates ladder pick from cleaned / basic-clean.
                (cleaned.clone(), Some(cleaned))
            }
        }
    } else {
        (after_selection.to_string(), None)
    };

    // Step 5: Produce requested formats. `Summary` also needs markdown
    // internally — the summary path feeds the markdown into the LLM and then
    // strips it from the response unless the caller also asked for markdown.
    let md = if formats.contains(&OutputFormat::Markdown)
        || formats.contains(&OutputFormat::Json)
        || formats.contains(&OutputFormat::Summary)
    {
        let primary_md = markdown::html_to_markdown(&content_html);
        let primary_quality = quality::analyze_md_only(&primary_md);

        // Skip alternates when a selector was explicitly used (short output is
        // intentional) or when the primary extraction is healthy.
        // Threshold 0.4 (not 0.6) — readability output that scores 0.4+ is
        // good enough; running alternates on it tends to swap in basic_clean
        // (whole-body) which boosts word count but reintroduces nav noise.
        if selected_html.is_some() || primary_quality.score > 0.4 {
            Some(primary_md)
        } else {
            let mut candidates: Vec<(&'static str, String, quality::Quality)> = Vec::new();

            // Alt 1: cleaned HTML (only_main_content path bypasses readability).
            if only_main_content && let Some(c) = cleaned_ref.as_ref() {
                let m = markdown::html_to_markdown(c);
                let q = quality::analyze_md_only(&m);
                candidates.push(("cleaned", m, q));
            }

            // Alt 2: basic clean without only_main_content (no readability narrowing).
            let basic_cleaned = clean::clean_html(raw_html, false, include_tags, exclude_tags)
                .unwrap_or_else(|_| raw_html.to_string());
            let basic_md = markdown::html_to_markdown(&basic_cleaned);
            let basic_q = quality::analyze_md_only(&basic_md);
            candidates.push(("basic_clean", basic_md, basic_q));

            // Alt 3: structural table/list extraction from raw HTML.
            if let Some(structural) = extract_tables_and_lists(raw_html) {
                let q = quality::analyze_md_only(&structural);
                candidates.push(("structural", structural, q));
            }

            // Alt 4: XHR/fetch JSON capture — recursively walk every captured
            // JSON body and gather long text fields. Useful when the article
            // body lives in an API response loaded after `loadEventFired`
            // (newsroom feeds, infinite-scroll, paywall-shielded prose).
            if let Some(xhr_md) = extract_xhr_text(captured_responses) {
                let q = quality::analyze_md_only(&xhr_md);
                candidates.push(("xhr_json", xhr_md, q));
            }

            // Alt 5: plaintext fallback.
            let plain_md = {
                let text = plaintext::html_to_plaintext(&content_html);
                if text.trim().is_empty() {
                    plaintext::html_to_plaintext(&basic_cleaned)
                } else {
                    text
                }
            };
            let plain_q = quality::analyze_md_only(&plain_md);
            candidates.push(("plaintext", plain_md, plain_q));

            // Include the primary at the head of the candidate list.
            candidates.insert(0, ("primary", primary_md, primary_quality));

            // Primary-biased pick: keep primary unless an alternate beats it by
            // a clear margin (0.15). Without this margin, basic_clean tends to
            // win simply by including more nav/footer words, which boosts its
            // word count but reintroduces noise the readability primary had
            // correctly excluded.
            const PRIMARY_MARGIN: f32 = 0.15;
            let primary_score = candidates[0].2.score;
            let chosen_idx = candidates
                .iter()
                .enumerate()
                .skip(1)
                .filter(|(_, c)| c.2.score >= primary_score + PRIMARY_MARGIN)
                .max_by(|(_, a), (_, b)| {
                    a.2.score
                        .partial_cmp(&b.2.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then(a.2.bytes.cmp(&b.2.bytes))
                })
                .map(|(i, _)| i)
                .unwrap_or(0);

            let names: Vec<&'static str> = candidates.iter().map(|c| c.0).collect();
            let scores: Vec<f32> = candidates.iter().map(|c| c.2.score).collect();
            let chosen_name = candidates[chosen_idx].0;
            tracing::debug!(
                strategies = ?names,
                scores = ?scores,
                chosen = %chosen_name,
                "quality-selected markdown extraction"
            );

            Some(candidates.swap_remove(chosen_idx).1)
        }
    } else {
        None
    };

    // News/blog templates frequently render the article H1 inside a `<header>`
    // sibling of the scored container, so readability drops it. Prepend the
    // metadata title (preferring the cleaner og:title) when it isn't already
    // present in the markdown — otherwise downstream recall scoring loses the
    // most important phrase on the page (the title itself).
    let md = md.map(|m| {
        if user_selected {
            return m;
        }
        let title = meta
            .og_title
            .as_deref()
            .or(meta.title.as_deref())
            .map(str::trim)
            .filter(|t| !t.is_empty());
        let Some(title) = title else { return m };
        // <title> commonly carries " | Site Name", " – Site Name", " — Site Name",
        // or " - Site Name" suffix; og:title is usually clean, but strip
        // defensively in either case. Pipe is rare inside real titles so we
        // split on the first occurrence (no whitespace required). En/em dash
        // and ASCII hyphen REQUIRE surrounding whitespace — a bare en dash
        // appears inside titles like "Northern Song Dynasty (960–1127)" and
        // must not split there; bare ASCII hyphens are common in compound
        // words. Dash splits are right-anchored so multi-segment titles like
        // "Foo – Bar – Site Name" reduce to "Foo – Bar" rather than "Foo".
        let core = title
            .split('|')
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(title);
        let core = core
            .rsplit_once(" – ")
            .map(|(l, _)| l.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or(core);
        let core = core
            .rsplit_once(" — ")
            .map(|(l, _)| l.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or(core);
        let core = core
            .rsplit_once(" - ")
            .map(|(l, _)| l.trim())
            .unwrap_or(core);
        if m.contains(core) || m.contains(title) {
            return m;
        }
        format!("# {core}\n\n{m}")
    });

    // When the extracted markdown is unusually short, append the page's
    // meta description / og:description — these are author-curated summaries
    // that frequently contain the article's key phrases, especially on:
    //   - Forum threads where readability picked one comment instead of the
    //     question (Discourse, vBulletin: meta description = first post body)
    //   - Listing pages whose readability rejection drops to a thin
    //     post-fallback ladder
    //   - Login-walled / app-shell pages where the SSR'd description is the
    //     only signal of what the page is about
    // Skip when the caller used a selector (intentional narrowness), the md
    // is already substantial, the description is short or already present,
    // or it duplicates the page title.
    let md = md.map(|m| {
        if user_selected {
            return m;
        }
        if m.len() >= 1500 {
            return m;
        }
        // Prefer whichever of `<meta name="description">` and
        // `<meta property="og:description">` is longer — Discourse and other
        // forum templates set the two to *different* posts (name=description
        // → original question, og:description → currently-displayed reply),
        // so picking the longer surfaces more unique content. When the two
        // diverge significantly (e.g. forum threads), append both so the
        // markdown captures the question *and* the highlighted reply.
        let name_desc = meta
            .description
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty());
        let og_desc = meta
            .og_description
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty());
        let combined = match (name_desc, og_desc) {
            (Some(a), Some(b)) if a == b => decode_basic_html_entities(a),
            (Some(a), Some(b)) => {
                let (longer, shorter) = if a.len() >= b.len() { (a, b) } else { (b, a) };
                let l = decode_basic_html_entities(longer);
                let s = decode_basic_html_entities(shorter);
                let probe_len = s.chars().take(60).map(char::len_utf8).sum::<usize>();
                let probe = &s[..probe_len.min(s.len())];
                if l.contains(probe) {
                    l
                } else {
                    format!("{l}\n\n{s}")
                }
            }
            (Some(a), None) | (None, Some(a)) => decode_basic_html_entities(a),
            (None, None) => return m,
        };
        let trimmed = combined.trim();
        // Defend against tagline-only descriptions (~30-50 chars) which add
        // no signal but pollute the leading content. Real article summaries
        // are nearly always >80 chars.
        if trimmed.chars().count() < 80 {
            return m;
        }
        let title_lc = meta
            .og_title
            .as_deref()
            .or(meta.title.as_deref())
            .map(|t| t.trim().to_lowercase())
            .unwrap_or_default();
        if !title_lc.is_empty() && trimmed.to_lowercase() == title_lc {
            return m;
        }
        // Cheap containment check — if the first ~120 chars of the
        // description already appear in the markdown, the body covers it.
        let probe_len = trimmed.chars().take(120).map(char::len_utf8).sum::<usize>();
        let probe = &trimmed[..probe_len.min(trimmed.len())];
        if m.contains(probe) {
            return m;
        }
        format!("{m}\n\n{trimmed}\n")
    });

    // Inline-list reflow: htmd emits each `<a>` inside a `<div>`/`<p>` wrapper as
    // its own paragraph, so a comma-separated label-and-link list (common on
    // company directories like dnb.com — `Industry: <a>X</a>, <a>Y</a>, Z`)
    // becomes `Industry:\u{00a0}\n\n[X], \n\n[Y], \n\nZ`. The runtime
    // `<a>` markup keeps the items inline; the surrounding blank lines are
    // a markdown-emit artefact that breaks substring matching across them.
    // Two passes: 1) NBSP → space (lossless: markdown has no NBSP semantics),
    // 2) collapse a blank line that sits between `, : )` punctuation and the
    // next inline link or comma-continuation paragraph.
    let md = md.map(reflow_inline_lists);

    let plain = if formats.contains(&OutputFormat::PlainText) {
        Some(plaintext::html_to_plaintext(&content_html))
    } else {
        None
    };

    let raw = if formats.contains(&OutputFormat::RawHtml) {
        Some(raw_html.to_string())
    } else {
        None
    };

    let html = if formats.contains(&OutputFormat::Html) {
        Some(content_html)
    } else {
        None
    };

    let links = if formats.contains(&OutputFormat::Links) {
        Some(readability::extract_links(raw_html, source_url))
    } else {
        None
    };

    // JSON extraction is handled asynchronously in scrape_url after extract() returns.
    let json = None;

    // Warn if filtering params are provided without a chunking strategy.
    let orphan_chunk_warning =
        if chunk_strategy.is_none() && (query.is_some() || filter_mode.is_some()) {
            Some(
                "'query' and 'filterMode' require 'chunkStrategy' to be set. \
             These parameters were ignored."
                    .to_string(),
            )
        } else {
            None
        };

    // Step 6: Chunk the markdown if a strategy is provided.
    let chunks = if let Some(strategy) = chunk_strategy
        && let Some(ref markdown_text) = md
        && !markdown_text.trim().is_empty()
    {
        let raw_chunks = chunking::chunk_text(markdown_text, strategy);

        // Step 7: Filter chunks by relevance if query + filter_mode are set.
        let chunk_results = if let (Some(q), Some(mode)) = (query, filter_mode)
            && !q.trim().is_empty()
            && !raw_chunks.is_empty()
        {
            filter::filter_chunks_scored(&raw_chunks, q, mode, top_k.unwrap_or(5))
                .into_iter()
                .map(|sc| ChunkResult {
                    content: sc.content,
                    score: Some(sc.score),
                    index: sc.index,
                })
                .collect::<Vec<_>>()
        } else {
            let mut results: Vec<_> = raw_chunks
                .into_iter()
                .enumerate()
                .map(|(i, c)| ChunkResult {
                    content: c,
                    score: None,
                    index: i,
                })
                .collect();
            if let Some(k) = top_k {
                results.truncate(k);
            }
            results
        };

        if chunk_results.is_empty() {
            None
        } else {
            Some(chunk_results)
        }
    } else {
        None
    };

    Ok(ScrapeData {
        markdown: md,
        html,
        raw_html: raw,
        plain_text: plain,
        links,
        json,
        summary: None,
        llm_usage: None,
        chunks,
        warning: orphan_chunk_warning,
        warnings,
        render_decision,
        credit_cost,
        metadata: PageMetadata {
            title: meta.title,
            description: meta.description,
            og_title: meta.og_title,
            og_description: meta.og_description,
            og_image: meta.og_image,
            canonical_url: meta.canonical_url,
            source_url: source_url.to_string(),
            language: meta.language,
            status_code,
            rendered_with,
            elapsed_ms,
            page_count: None,
            source_filename: None,
        },
        debug_extraction: None,
        // Populated post-extract by the caller (single.rs / crawl.rs) from
        // FetchResult.content_type; change_tracking + screenshot are set there too.
        content_type: None,
        change_tracking: None,
        screenshot: None,
    })
}

/// Apply CSS selector or XPath to narrow HTML content.
/// Returns None if no selector is set or no match is found.
fn apply_selector(html: &str, css: Option<&str>, xpath: Option<&str>) -> CrwResult<Option<String>> {
    if let Some(sel) = css {
        let result = selector::extract_by_css(html, sel).map_err(CrwError::ExtractionError)?;
        if result.is_some() {
            return Ok(result);
        }
    }
    if let Some(xp) = xpath
        && let Some(texts) =
            selector::extract_by_xpath(html, xp).map_err(CrwError::ExtractionError)?
    {
        let wrapped = texts
            .into_iter()
            .map(|text| {
                let escaped = text
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                format!("<div>{escaped}</div>")
            })
            .collect::<Vec<_>>()
            .join("\n");
        return Ok(Some(wrapped));
    }
    Ok(None)
}

/// Walk the raw HTML for substantial `<table>` (≥2 data rows) and
/// `<ul>/<ol>` (≥5 items) elements, render each to markdown, and return
/// the concatenation. Returns `None` if no qualifying structure is found.
///
/// This exists as a last-ditch fallback: readability and the htmd-on-cleaned
/// path treat tabular and list-only pages (county finance reports, job
/// listings, niche product catalogs) as navigation noise. By pulling those
/// structures out of the raw DOM we surface real content that would
/// otherwise be reported as thin.
/// Walk the captured XHR/fetch JSON responses and harvest long text fields.
/// Each response is parsed as JSON; every string value with at least
/// `MIN_FIELD_LEN` characters is appended (deduplicated). Returned as a
/// markdown-ish body (paragraph-separated). `None` if total content is
/// too small to be useful.
fn extract_xhr_text(captured: &[CapturedNetworkResponse]) -> Option<String> {
    const MIN_FIELD_LEN: usize = 120;
    const MIN_TOTAL_LEN: usize = 400;

    if captured.is_empty() {
        return None;
    }
    let mut paragraphs: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for resp in captured {
        let body = match resp.body.as_deref() {
            Some(b) if !b.is_empty() => b,
            _ => continue,
        };
        let value: serde_json::Value = match serde_json::from_str(body) {
            Ok(v) => v,
            Err(_) => continue,
        };
        walk_json_strings(&value, &mut |s| {
            if s.len() >= MIN_FIELD_LEN && seen.insert(s.to_string()) {
                paragraphs.push(s.to_string());
            }
        });
    }

    if paragraphs.is_empty() {
        return None;
    }
    let joined = paragraphs.join("\n\n");
    if joined.len() < MIN_TOTAL_LEN {
        return None;
    }
    Some(joined)
}

fn walk_json_strings(value: &serde_json::Value, on_string: &mut dyn FnMut(&str)) {
    match value {
        serde_json::Value::String(s) => {
            // Skip URLs, IDs, dates, and HTML tag fragments — keep prose only.
            let trimmed = s.trim();
            if trimmed.starts_with("http://")
                || trimmed.starts_with("https://")
                || trimmed.starts_with('/')
                || trimmed.starts_with('<')
            {
                return;
            }
            on_string(trimmed);
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                walk_json_strings(v, on_string);
            }
        }
        serde_json::Value::Object(map) => {
            for (_, v) in map {
                walk_json_strings(v, on_string);
            }
        }
        _ => {}
    }
}

fn extract_tables_and_lists(html: &str) -> Option<String> {
    use scraper::{Html, Selector};

    let doc = Html::parse_document(html);
    let table_sel = Selector::parse("table").ok()?;
    let list_sel = Selector::parse("ul, ol").ok()?;
    let row_sel = Selector::parse("tr").ok()?;
    let item_sel = Selector::parse("li").ok()?;

    let mut chunks: Vec<String> = Vec::new();

    for table in doc.select(&table_sel) {
        if table.select(&row_sel).count() < 2 {
            continue;
        }
        let html_chunk = table.html();
        let md = markdown::html_to_markdown(&html_chunk);
        if md.trim().len() >= 40 {
            chunks.push(md);
        }
    }

    for list in doc.select(&list_sel) {
        if list.select(&item_sel).count() < 5 {
            continue;
        }
        // Skip nav/footer lists — those are usually identifiable by ancestor
        // tag and would otherwise drown out real content.
        let in_nav = list
            .ancestors()
            .filter_map(scraper::ElementRef::wrap)
            .any(|el| {
                let n = el.value().name();
                n == "nav" || n == "footer" || n == "header"
            });
        if in_nav {
            continue;
        }
        let html_chunk = list.html();
        let md = markdown::html_to_markdown(&html_chunk);
        if md.trim().len() >= 40 {
            chunks.push(md);
        }
    }

    if chunks.is_empty() {
        return None;
    }
    Some(chunks.join("\n\n"))
}

#[cfg(test)]
mod table_list_fallback_tests {
    use super::*;

    #[test]
    fn extracts_two_row_table() {
        let html = "<html><body><nav>x</nav><table>\
            <tr><th>Name</th><th>Value</th></tr>\
            <tr><td>Alpha</td><td>1</td></tr>\
            <tr><td>Bravo</td><td>2</td></tr>\
            </table></body></html>";
        let md = extract_tables_and_lists(html).expect("table should extract");
        assert!(md.contains("Alpha"));
        assert!(md.contains("Bravo"));
    }

    #[test]
    fn skips_short_table() {
        let html = "<table><tr><td>only</td></tr></table>";
        assert!(extract_tables_and_lists(html).is_none());
    }

    #[test]
    fn skips_nav_list() {
        let html = "<nav><ul>\
            <li>a</li><li>b</li><li>c</li><li>d</li><li>e</li><li>f</li>\
            </ul></nav>";
        assert!(extract_tables_and_lists(html).is_none());
    }

    #[test]
    fn extracts_long_list() {
        let html = "<main><ul>\
            <li>Job A</li><li>Job B</li><li>Job C</li>\
            <li>Job D</li><li>Job E</li><li>Job F</li>\
            </ul></main>";
        let md = extract_tables_and_lists(html).expect("list should extract");
        assert!(md.contains("Job A"));
        assert!(md.contains("Job F"));
    }
}
