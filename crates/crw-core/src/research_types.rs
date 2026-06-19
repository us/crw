//! Firecrawl-Research-API-compatible response types for `/v1/search/research/*`.
//!
//! Field names + shapes mirror Firecrawl's v2 research SDK so that the Firecrawl
//! SDK/CLI works drop-in against our base URL. Two distinct paper shapes:
//! [`ResearchPaperResult`] (search / similar — has `score` + optional `signals`,
//! no authors/dates) vs [`ResearchPaperMeta`] (inspect — has authors/categories/
//! dates, no score/signals). `abstract` is a Rust keyword, renamed at the field.
//!
//! `paperId` is our canonical id (the OpenAlex work id, URL-safe); `primaryId`
//! is the preferred prefixed source id (`"arxiv:2105.05233"`); `ids` carries the
//! PREFIX-LESS source ids (`{"arxiv": ["2105.05233"]}`).

use serde::Serialize;
use std::collections::HashMap;

/// A ranked paper in `papers` (search) and `similar` results.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchPaperResult {
    pub paper_id: String,
    pub primary_id: String,
    pub ids: HashMap<String, Vec<String>>,
    pub title: String,
    #[serde(rename = "abstract", skip_serializing_if = "Option::is_none")]
    pub abstract_: Option<String>,
    pub score: f64,
    /// Omitted entirely when we can't compute Firecrawl's structural graph
    /// signals (it's optional in their SDK; partial nulls would break it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signals: Option<ResearchSignals>,
}

/// Optional ranking signals. If present, ALL fields are numbers (never null).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchSignals {
    pub structural: f64,
    pub semantic: f64,
    pub article_rank: f64,
    pub seed_overlap: f64,
}

/// Paper metadata returned by `GET /papers/{id}` (no `score`/`signals`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchPaperMeta {
    pub paper_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ids: Option<HashMap<String, Vec<String>>>,
    pub title: String,
    #[serde(rename = "abstract", skip_serializing_if = "Option::is_none")]
    pub abstract_: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authors: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_date: Option<String>,
}

/// One full-text passage answering a `?query` on `GET /papers/{id}`.
#[derive(Debug, Clone, Serialize)]
pub struct ResearchPassage {
    pub text: String,
    pub score: f64,
}

/// A GitHub history / README hit from `GET /search/research/github`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchGithubItem {
    pub result_type: String,
    pub repo: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readme_url: Option<String>,
    pub title: String,
    pub snippet: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_md: Option<String>,
}

// --- response wrappers (Firecrawl shape: {success, ...}) ---

#[derive(Debug, Clone, Serialize)]
pub struct PapersResponse {
    pub success: bool,
    pub results: Vec<ResearchPaperResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PaperMetaResponse {
    pub success: bool,
    pub paper: ResearchPaperMeta,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadPaperResponse {
    pub success: bool,
    pub paper: ResearchPaperMeta,
    pub paper_id: String,
    pub query: String,
    pub passages: Vec<ResearchPassage>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimilarResponse {
    pub success: bool,
    pub results: Vec<ResearchPaperResult>,
    pub pool_size: usize,
    pub truncated: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GithubResponse {
    pub success: bool,
    pub results: Vec<ResearchGithubItem>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paper_result_serializes_firecrawl_shape() {
        let p = ResearchPaperResult {
            paper_id: "W2105".to_string(),
            primary_id: "arxiv:2105.05233".to_string(),
            ids: HashMap::from([("arxiv".to_string(), vec!["2105.05233".to_string()])]),
            title: "Diffusion Models".to_string(),
            abstract_: Some("We present...".to_string()),
            score: 0.42,
            signals: None,
        };
        let v = serde_json::to_value(&p).unwrap();
        // camelCase keys + prefix-less ids + "abstract" + signals omitted
        assert_eq!(v["paperId"], "W2105");
        assert_eq!(v["primaryId"], "arxiv:2105.05233");
        assert_eq!(v["ids"]["arxiv"][0], "2105.05233");
        assert_eq!(v["abstract"], "We present...");
        assert!(
            v.get("signals").is_none(),
            "signals must be omitted, not null"
        );
    }

    #[test]
    fn meta_omits_score_and_empty_fields() {
        let m = ResearchPaperMeta {
            paper_id: "W1".to_string(),
            ids: None,
            title: "T".to_string(),
            abstract_: None,
            authors: None,
            categories: None,
            created_date: None,
            update_date: None,
        };
        let v = serde_json::to_value(&m).unwrap();
        assert!(v.get("score").is_none());
        assert!(v.get("authors").is_none());
        assert_eq!(v["paperId"], "W1");
    }
}
