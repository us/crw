//! Evidence & provenance primitives shared across search/extract.
//!
//! These types give every answer, structured field, and ranked result a
//! traceable basis: which source it came from, the exact span, and a content
//! hash so the span can be re-verified against the canonical scraped markdown.
//! All offsets are **char** indices into the normalized markdown that
//! `crw_diff::snapshot::hash_markdown` hashes (CRLF/trailing-WS/blank-run
//! normalized), so offsets and hashes stay consistent across the pipeline.
//!
//! Everything here is **additive and serde-stable**: `camelCase` on the wire,
//! `skip_serializing_if` on every optional/empty field, so attaching evidence
//! to an existing response never changes the bytes a client that ignores it
//! already parses. Wiring lands incrementally (Phase 1a: search highlights +
//! per-source evidence; Phase 2b: per-field extraction basis).
//!
//! Deferred on purpose (no consumer until Phase 3 durable jobs / budget
//! accounting, and `Usage` would otherwise duplicate [`crate::types::LlmUsage`]):
//! `RunBudget`, a pipeline-superset `Usage`, and a structured `ApiWarning`.
//! Add them in the step that first enforces a budget or emits a coded warning.

use serde::{Deserialize, Serialize};

/// Qualitative confidence, mirroring [`crate::types::ChangeConfidence`] so the
/// extraction and change-tracking layers speak the same vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfidenceLevel {
    Low,
    Medium,
    High,
}

/// A scored span of source text supporting an answer or a ranked result.
///
/// `char_start`/`char_end` index into the canonical normalized markdown of the
/// source identified by `source_hash`; `score` is the span's relevance to the
/// scoring query (objective), higher is better.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Highlight {
    pub text: String,
    pub score: f64,
    pub char_start: usize,
    pub char_end: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
}

/// A citation backing an extracted value: the source, the quoted excerpt, and
/// (when known) the exact span within that source's canonical markdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceCitation {
    pub url: String,
    pub title: String,
    pub excerpt: String,
    pub source_hash: String,
    /// Which text the offsets index into, e.g. `"markdown"` (canonical) — lets a
    /// consumer pick the right canonical-source-text store to re-verify against.
    pub source_text_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub char_start: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub char_end: Option<usize>,
}

/// Per-source evidence attached to a search answer: one entry per source that
/// contributed, carrying the spans that informed the synthesized answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceEvidence {
    pub url: String,
    pub title: String,
    pub position: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub highlights: Vec<Highlight>,
}

/// The basis for a single extracted structured field: its value, why the model
/// chose it, and the citations that support it. `basis_version` lets consumers
/// gate on the schema as it evolves.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Basis {
    pub basis_version: u8,
    pub field: String,
    pub value: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<ConfidenceLevel>,
    pub reasoning: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub citations: Vec<EvidenceCitation>,
}

/// Caller-supplied source-selection policy: which domains/freshness/types a
/// search or extract may draw from. A **struct of filters** (not an enum) — all
/// fields default empty/false, so an absent policy is "no constraint".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePolicy {
    #[serde(default)]
    pub include_domains: Vec<String>,
    #[serde(default)]
    pub exclude_domains: Vec<String>,
    #[serde(default)]
    pub prefer_domains: Vec<String>,
    #[serde(default)]
    pub allow_subdomains: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_after: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age_hours: Option<u32>,
    #[serde(default)]
    pub force_live: bool,
    #[serde(default)]
    pub source_types: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // Wire-contract guard: camelCase keys, and empty `highlights` is omitted so
    // attaching evidence to an existing response is byte-invisible to clients
    // that don't read it (the additive-safety the conformance gate relies on).
    #[test]
    fn source_evidence_is_camelcase_and_skips_empty() {
        let ev = SourceEvidence {
            url: "https://example.com".into(),
            title: "Example".into(),
            position: 1,
            highlights: vec![],
        };
        let j = serde_json::to_value(&ev).unwrap();
        assert_eq!(j["url"], "https://example.com");
        assert!(
            j.get("highlights").is_none(),
            "empty highlights must be skipped"
        );

        let hl = Highlight {
            text: "x".into(),
            score: 0.5,
            char_start: 0,
            char_end: 1,
            source_hash: None,
        };
        let jh = serde_json::to_value(&hl).unwrap();
        assert!(
            jh.get("charStart").is_some(),
            "char_start must serialize as charStart"
        );
        assert!(
            jh.get("sourceHash").is_none(),
            "None source_hash must be skipped"
        );
    }

    #[test]
    fn confidence_level_is_lowercase() {
        assert_eq!(
            serde_json::to_value(ConfidenceLevel::High).unwrap(),
            serde_json::json!("high")
        );
    }

    #[test]
    fn source_policy_default_is_unconstrained() {
        let j = serde_json::to_value(SourcePolicy::default()).unwrap();
        // Empty option fields skipped; vec/bool fields present-but-empty/false.
        assert!(j.get("publishedAfter").is_none());
        assert_eq!(j["allowSubdomains"], false);
        assert_eq!(j["includeDomains"], serde_json::json!([]));
    }
}
