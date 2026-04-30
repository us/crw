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

pub mod chunking;
pub mod clean;
pub mod filter;
pub mod markdown;
#[cfg(feature = "pdf")]
pub mod pdf;
pub mod plaintext;
pub mod readability;
pub mod selector;
pub mod structured;

use crw_core::error::{CrwError, CrwResult};
use crw_core::types::{
    ChunkResult, ChunkStrategy, FilterMode, OutputFormat, PageMetadata, RenderDecision, ScrapeData,
};

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
}

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
    } = opts;

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
        let main = readability::extract_main_content(after_selection);
        // Re-clean: readability may have selected a broad container (e.g. <article>)
        // that still contains noise elements (infobox, navbox, catlinks, etc.)
        let re_cleaned = clean::clean_html(&main, true, &[], &[]).unwrap_or(main);
        (re_cleaned, Some(cleaned))
    } else {
        (after_selection.to_string(), None)
    };

    // Step 5: Produce requested formats.
    let md = if formats.contains(&OutputFormat::Markdown) || formats.contains(&OutputFormat::Json) {
        let md = markdown::html_to_markdown(&content_html);
        // Trigger fallback if markdown is empty OR suspiciously short relative to HTML.
        // Skip fallback when a CSS/XPath selector was explicitly used — short output is intentional.
        let md_too_short =
            selected_html.is_none() && md.trim().len() < 100 && raw_html.len() > 5000;
        if md_too_short {
            let fallback_md = if only_main_content && selected_html.is_none() {
                // Try both fallbacks and pick whichever produced more content.
                let from_cleaned = cleaned_ref
                    .as_ref()
                    .map(|c| markdown::html_to_markdown(c))
                    .unwrap_or_default();

                let basic_md = {
                    let basic_cleaned =
                        clean::clean_html(raw_html, false, include_tags, exclude_tags)
                            .unwrap_or_else(|_| raw_html.to_string());
                    markdown::html_to_markdown(&basic_cleaned)
                };

                // Pick whichever produced more content
                if from_cleaned.trim().len() >= basic_md.trim().len() {
                    from_cleaned
                } else {
                    basic_md
                }
            } else {
                markdown::html_to_markdown(raw_html)
            };

            let fallback_too_short = fallback_md.trim().len() < 100 && raw_html.len() > 5000;
            if fallback_too_short {
                // Last-resort structural fallback: some pages (county budget
                // listings, sheriff vacancies, niche product catalogs) carry
                // their entire payload inside a single <table> or a long
                // <ul>/<ol> that readability discards as "navigation". If we
                // can pull such structures out of the raw HTML and they
                // dominate the page, prefer them over the plaintext path.
                let structural = extract_tables_and_lists(raw_html);
                let plain_text_fb = {
                    let text = plaintext::html_to_plaintext(&content_html);
                    if text.trim().is_empty() {
                        let basic_cleaned =
                            clean::clean_html(raw_html, false, include_tags, exclude_tags)
                                .unwrap_or_else(|_| raw_html.to_string());
                        plaintext::html_to_plaintext(&basic_cleaned)
                    } else {
                        text
                    }
                };
                match structural {
                    Some(s) if s.trim().len() > plain_text_fb.trim().len() => Some(s),
                    _ => Some(plain_text_fb),
                }
            } else {
                Some(fallback_md)
            }
        } else {
            Some(md)
        }
    } else {
        None
    };

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
        },
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
