//! Evidence & provenance primitives shared across search/extract.
//!
//! These types give every answer, structured field, and ranked result a
//! traceable basis: which source it came from, the supporting span, and a
//! content hash so the span can be re-verified against the canonical source
//! text it was drawn from.
//!
//! **Two canonical source texts, and they are not the same bytes.** Which one a
//! hash covers is named by `source_text_kind`:
//!
//! - [`Highlight::source_hash`] (search) is the bare hex hash of the **full**
//!   normalized markdown, as produced by `crw_diff::snapshot::hash_markdown`
//!   (CRLF/trailing-WS/blank-run normalized). Its `char_start`/`char_end`
//!   index into that text.
//! - [`EvidenceCitation::source_hash`] (extraction basis) is
//!   `"sha256:" + hex(sha256(bytes))` over the **truncated markdown actually
//!   sent to the LLM** — the exact bytes after the clean/markdown pipeline and
//!   after `crw_extract::structured`'s input-byte clip. `source_text_kind` is
//!   `"llmInput"` for these. They do not collide with the full-markdown hash,
//!   and `crate::types::ScrapeData::source_hash` (the full-markdown one) is
//!   still surfaced alongside, so a caller can correlate with change-tracking
//!   and re-scrape.
//!
//! Search evidence ([`Highlight`], [`SourceEvidence`]) is **additive and
//! byte-invisible**: `camelCase` on the wire with `skip_serializing_if` on
//! every optional/empty field, so attaching it to an existing response never
//! changes the bytes a client that ignores it already parses.
//!
//! [`Basis`] is deliberately the **opposite**: `value` and `citations` always
//! serialize, even as `null` and `[]`. An absent key reads as *unknown*; an
//! explicit `null`/`[]` reads as *none*, and that distinction is the honesty
//! contract (see [`FieldStatus`]). A `Basis` is never attached to a response
//! that did not ask for one, so there is nothing to stay byte-invisible to.
//!
//! Deferred on purpose (no consumer until durable jobs / budget accounting, and
//! `Usage` would otherwise duplicate [`crate::types::LlmUsage`]): `RunBudget`,
//! a pipeline-superset `Usage`, and a structured `ApiWarning`.

use serde::{Deserialize, Serialize};

/// Wire-format version of [`Basis`]. Bumped on any change to the basis wire
/// shape, so a consumer can gate on it. See `docs/adr/0001-basis-wire-format.md`.
pub const BASIS_VERSION: u8 = 1;

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
/// (when known) the exact span within that source's canonical text.
///
/// Every field here is **server-produced**. `url` is the document the server
/// fetched, `source_hash` is the server's own hash of the bytes it sent to the
/// model, and `excerpt` only survives if it passed the deterministic checks in
/// `crw_extract::structured`. A model-supplied url, hash or title is never
/// echoed onto the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceCitation {
    pub url: String,
    /// Server-stamped page title. `None` in basis v1; never model-supplied (a
    /// model-supplied title would be a synthesized attribution).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The supporting span, verbatim from the canonical source text. `None`
    /// means the span could not be established — see [`FieldStatus::Unverified`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    /// Hash of the canonical source text this citation is drawn from. For
    /// extraction basis: `"sha256:" + hex(sha256(llm-input bytes))`. See the
    /// module header — this is NOT the full-markdown hash.
    pub source_hash: String,
    /// Which text `source_hash` covers and any offsets index into. `"llmInput"`
    /// for extraction basis (the truncated markdown sent to the model);
    /// `"markdown"` for the full normalized markdown. Lets a consumer pick the
    /// right canonical-source-text store to re-verify against.
    pub source_text_kind: String,
    /// Always `None` in basis v1 (offsets are out of scope; the excerpt is
    /// verified by containment, not by index).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub char_start: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub char_end: Option<usize>,
}

/// How well an extracted field is attributed to its source. Wire: `camelCase`
/// (`supported | unverified | unsupported | notFound`).
///
/// The contract, per [`Basis`]:
///
/// | status | `value` | `citations` | `citations[0].excerpt` |
/// | --- | --- | --- | --- |
/// | `Supported` | non-null | exactly 1 | present, and it carries the value |
/// | `Unverified` | non-null | exactly 1 | `None` |
/// | `Unsupported` | non-null | empty | n/a |
/// | `NotFound` | **null** | empty | n/a |
///
/// `Unverified` means: *the document is real and attributable; the span within
/// it is not established.* `Unsupported` means: *no attribution survives; the
/// value is retained but explicitly untrusted.*
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldStatus {
    Supported,
    Unverified,
    Unsupported,
    NotFound,
}

/// A coded warning explaining why a field's attribution was downgraded.
///
/// `code` is a closed, crw-owned set; `field` is the caller's own schema
/// property name. Never upstream text: this rides on responses that may be
/// persisted and read back by a customer, so an unsanitized upstream string
/// here would be a durable leak.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BasisWarning {
    pub field: String,
    pub code: String,
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

/// The basis for a single extracted structured field: its value, how well that
/// value is attributed, and the citation supporting it.
///
/// Emitted only for **top-level scalar** schema properties. `basis_version`
/// ([`BASIS_VERSION`]) lets consumers gate on the shape as it evolves.
///
/// `value` and `citations` intentionally always serialize (no
/// `skip_serializing_if`), unlike the search-evidence types in this module: an
/// absent key reads as *unknown*, an explicit `null`/`[]` reads as *none*, and
/// [`FieldStatus`]'s contract rests on that distinction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Basis {
    pub basis_version: u8,
    pub field: String,
    /// The extracted value. `null` **iff** `status` is [`FieldStatus::NotFound`].
    /// Always the schema-validated value from the response body — never
    /// rewritten to match what the model claimed in its basis.
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    pub status: FieldStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<ConfidenceLevel>,
    /// Not requested from the model in basis v1 (it carries no invariant and
    /// costs output tokens on every field). Always `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// 0 or 1 citations in basis v1. Empty **iff** `status` is
    /// [`FieldStatus::Unsupported`] or [`FieldStatus::NotFound`].
    #[serde(default)]
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

    // The basis wire contract, guarded. `value: null` and `citations: []` must
    // be PRESENT, not skipped: absent reads as "unknown", explicit reads as
    // "none", and FieldStatus's table depends on telling those apart.
    #[test]
    fn basis_null_value_and_empty_citations_are_visible() {
        let b = Basis {
            basis_version: BASIS_VERSION,
            field: "price".into(),
            value: None,
            status: FieldStatus::NotFound,
            confidence: None,
            reasoning: None,
            citations: vec![],
        };
        let j = serde_json::to_value(&b).unwrap();
        assert_eq!(j["basisVersion"], 1);
        assert!(
            j.get("value").is_some_and(serde_json::Value::is_null),
            "notFound must serialize an explicit null value, got: {j}"
        );
        assert_eq!(
            j["citations"],
            serde_json::json!([]),
            "empty citations must serialize as [], not be skipped"
        );
        assert!(j.get("reasoning").is_none(), "None reasoning is skipped");
    }

    #[test]
    fn field_status_is_camelcase_on_the_wire() {
        let j = |s: FieldStatus| serde_json::to_value(s).unwrap();
        assert_eq!(j(FieldStatus::Supported), serde_json::json!("supported"));
        assert_eq!(j(FieldStatus::Unverified), serde_json::json!("unverified"));
        assert_eq!(
            j(FieldStatus::Unsupported),
            serde_json::json!("unsupported")
        );
        assert_eq!(j(FieldStatus::NotFound), serde_json::json!("notFound"));
    }

    #[test]
    fn evidence_citation_is_camelcase() {
        let c = EvidenceCitation {
            url: "https://example.com".into(),
            title: None,
            excerpt: Some("Price: $19.99".into()),
            source_hash: "sha256:ab".into(),
            source_text_kind: "llmInput".into(),
            char_start: None,
            char_end: None,
        };
        let j = serde_json::to_value(&c).unwrap();
        assert_eq!(j["sourceHash"], "sha256:ab");
        assert_eq!(j["sourceTextKind"], "llmInput");
        assert_eq!(j["excerpt"], "Price: $19.99");
        assert!(j.get("title").is_none(), "None title is skipped");
        assert!(j.get("charStart").is_none(), "v1 emits no offsets");
    }

    #[test]
    fn basis_warning_is_camelcase() {
        let w = BasisWarning {
            field: "price".into(),
            code: "basis_value_mismatch".into(),
        };
        let j = serde_json::to_value(&w).unwrap();
        assert_eq!(j["field"], "price");
        assert_eq!(j["code"], "basis_value_mismatch");
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
