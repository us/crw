//! Translate a public [`SearchRequest`] into SearXNG query parameters.
//!
//! Direct port of `crw-saas/src/lib/search-params.ts:mapToSearxngParams`.
//! Fixture-tested in `tests/transform_tests.rs` for byte-for-byte parity
//! with the SaaS implementation.
//!
//! [`SearchRequest`]: crw_core::types::SearchRequest

use crw_core::config::SearchConfig;
use crw_core::types::{SearchCategory, SearchRequest};

/// Owned representation of the SearXNG query parameters we send. The client
/// constructs the URL-encoded form from these fields; this struct stays
/// JSON-neutral so it's easy to assert on in tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearxngParams {
    pub q: String,
    pub categories: Option<String>,
    pub language: Option<String>,
    pub time_range: Option<String>,
    pub engines: Option<String>,
    pub pageno: Option<u32>,
    pub safesearch: Option<u8>,
}

/// Map the public [`SearchRequest`] to SearXNG query parameters.
///
/// Replicates SaaS quirks faithfully:
/// - `categories: ["pdf"]` appends ` filetype:pdf` to the query (not engines).
/// - `categories: ["github"]` adds the `github` engine.
/// - `categories: ["research"]` expands to `arxiv,crossref,google scholar,semantic scholar`
///   (configurable via `[search].research_engines`).
/// - `tbs: "qdr:h"` is mapped to SearXNG `time_range=day` — SearXNG has no
///   hour granularity. (See `SearchTimeFilter::searxng_time_range`.)
///
/// [`SearchRequest`]: crw_core::types::SearchRequest
pub fn map_to_searxng_params(req: &SearchRequest, config: &SearchConfig) -> SearxngParams {
    let mut query = req.query.clone();
    let mut engines: Vec<String> = Vec::new();

    if let Some(cats) = &req.categories {
        for cat in cats {
            match cat {
                SearchCategory::Pdf => {
                    query.push_str(" filetype:pdf");
                }
                SearchCategory::Github => {
                    engines.extend(config.github_engines.iter().cloned());
                }
                SearchCategory::Research => {
                    engines.extend(config.research_engines.iter().cloned());
                }
            }
        }
    }

    let categories = req.sources.as_ref().map(|srcs| {
        srcs.iter()
            .map(|s| s.searxng_category())
            .collect::<Vec<_>>()
            .join(",")
    });

    let time_range = req.tbs.map(|t| t.searxng_time_range().to_string());
    let language = req.lang.clone().filter(|s| !s.is_empty());
    let engines = if engines.is_empty() {
        None
    } else {
        Some(engines.join(","))
    };

    SearxngParams {
        q: query,
        categories,
        language,
        time_range,
        engines,
        pageno: None,
        safesearch: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crw_core::types::{SearchSource, SearchTimeFilter};

    fn cfg() -> SearchConfig {
        SearchConfig::default()
    }

    fn req(q: &str) -> SearchRequest {
        SearchRequest {
            query: q.into(),
            limit: None,
            lang: None,
            tbs: None,
            sources: None,
            categories: None,
            scrape_options: None,
            summarize_results: None,
            answer: None,
            answer_top_n: None,
            max_chars_per_source: None,
            llm_api_key: None,
            llm_provider: None,
            llm_model: None,
            base_url: None,
            summary_prompt: None,
            answer_prompt: None,
        }
    }

    #[test]
    fn plain_query_is_passed_through() {
        let p = map_to_searxng_params(&req("rust async"), &cfg());
        assert_eq!(p.q, "rust async");
        assert!(p.categories.is_none());
        assert!(p.engines.is_none());
        assert!(p.time_range.is_none());
    }

    #[test]
    fn pdf_category_modifies_query_only() {
        let mut r = req("rust");
        r.categories = Some(vec![SearchCategory::Pdf]);
        let p = map_to_searxng_params(&r, &cfg());
        assert_eq!(p.q, "rust filetype:pdf");
        assert!(p.engines.is_none());
    }

    #[test]
    fn github_category_sets_engines() {
        let mut r = req("rust");
        r.categories = Some(vec![SearchCategory::Github]);
        let p = map_to_searxng_params(&r, &cfg());
        assert_eq!(p.engines.as_deref(), Some("github"));
        assert_eq!(p.q, "rust");
    }

    #[test]
    fn research_category_expands_to_default_engines() {
        let mut r = req("transformers");
        r.categories = Some(vec![SearchCategory::Research]);
        let p = map_to_searxng_params(&r, &cfg());
        assert_eq!(
            p.engines.as_deref(),
            Some("arxiv,crossref,google scholar,semantic scholar")
        );
    }

    #[test]
    fn sources_join_to_searxng_categories() {
        let mut r = req("rust");
        r.sources = Some(vec![SearchSource::Web, SearchSource::News]);
        let p = map_to_searxng_params(&r, &cfg());
        assert_eq!(p.categories.as_deref(), Some("general,news"));
    }

    #[test]
    fn tbs_hour_collapses_to_day() {
        let mut r = req("rust");
        r.tbs = Some(SearchTimeFilter::Hour);
        let p = map_to_searxng_params(&r, &cfg());
        assert_eq!(p.time_range.as_deref(), Some("day"));
    }

    #[test]
    fn tbs_year_maps_to_year() {
        let mut r = req("rust");
        r.tbs = Some(SearchTimeFilter::Year);
        let p = map_to_searxng_params(&r, &cfg());
        assert_eq!(p.time_range.as_deref(), Some("year"));
    }

    #[test]
    fn empty_lang_drops_to_none() {
        let mut r = req("rust");
        r.lang = Some(String::new());
        let p = map_to_searxng_params(&r, &cfg());
        assert!(p.language.is_none());
    }

    #[test]
    fn pdf_plus_github_combine() {
        let mut r = req("memory");
        r.categories = Some(vec![SearchCategory::Pdf, SearchCategory::Github]);
        let p = map_to_searxng_params(&r, &cfg());
        assert_eq!(p.q, "memory filetype:pdf");
        assert_eq!(p.engines.as_deref(), Some("github"));
    }
}
