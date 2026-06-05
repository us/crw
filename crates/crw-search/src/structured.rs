//! Structured facts from SearXNG's `infoboxes[]` / `answers[]` arrays.
//!
//! SearXNG's `format=json` envelope returns five arrays; the Wikidata/Wikipedia
//! engines emit their knowledge-panel data (entity attributes like
//! `religion → X`, `capital → Y`) into `infoboxes[]` / `answers[]`, NOT into
//! `results[]`. The normal transform path reads only `results[]`, so these
//! already-retrieved structured facts were silently discarded (W0). This module
//! parses them so the answer path can pin them as a high-trust source. They are
//! still UNTRUSTED-wrapped by the synthesizer — this only widens the evidence,
//! it does not bypass the safety wrapper.

use crate::client::SearxngResponse;
use serde_json::Value;

/// A structured fact extracted from an infobox or a direct answer. `attributes`
/// are the infobox key/value rows (e.g. `("religion", "Sunni Islam")`).
#[derive(Debug, Clone, PartialEq)]
pub struct StructuredFact {
    pub title: String,
    pub url: String,
    pub content: String,
    pub attributes: Vec<(String, String)>,
    /// Always true — marks this as a pinned structured source so a later
    /// rerank-bypass (W1) can key off the flag, not the domain.
    pub is_structured_source: bool,
}

impl StructuredFact {
    /// Compact markdown body for the answer-path source (title is carried
    /// separately in the `Source` tuple).
    pub fn to_markdown(&self) -> String {
        let mut s = String::new();
        if !self.content.is_empty() {
            s.push_str(&self.content);
            s.push('\n');
        }
        for (k, v) in &self.attributes {
            s.push_str("- ");
            s.push_str(k);
            s.push_str(": ");
            s.push_str(v);
            s.push('\n');
        }
        s.trim_end().to_string()
    }
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
}

/// Parse `infoboxes[]` + `answers[]` into structured facts. Defensive: every
/// field is optional, malformed/empty entries are skipped (degrade to nothing).
pub fn structured_facts(resp: &SearxngResponse) -> Vec<StructuredFact> {
    let mut out = Vec::new();

    for ib in &resp.infoboxes {
        let title = str_field(ib, "infobox").unwrap_or_default();
        let url = str_field(ib, "id").unwrap_or_default();
        let content = str_field(ib, "content").unwrap_or_default();
        let mut attributes = Vec::new();
        if let Some(arr) = ib.get("attributes").and_then(|x| x.as_array()) {
            for a in arr {
                if let (Some(label), Some(value)) = (str_field(a, "label"), str_field(a, "value")) {
                    attributes.push((label, value));
                }
            }
        }
        // Nothing useful to feed the synthesizer.
        if content.is_empty() && attributes.is_empty() {
            continue;
        }
        out.push(StructuredFact {
            title: if title.is_empty() {
                "Structured fact".to_string()
            } else {
                title
            },
            url,
            content,
            attributes,
            is_structured_source: true,
        });
    }

    // `answers[]` entries are either a bare string or `{answer, url}`.
    for ans in &resp.answers {
        let (content, url) = match ans {
            Value::String(t) => (t.trim().to_string(), String::new()),
            Value::Object(_) => (
                str_field(ans, "answer").unwrap_or_default(),
                str_field(ans, "url").unwrap_or_default(),
            ),
            _ => continue,
        };
        if content.is_empty() {
            continue;
        }
        out.push(StructuredFact {
            title: "Direct answer".to_string(),
            url,
            content,
            attributes: Vec::new(),
            is_structured_source: true,
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn resp_with(infoboxes: Vec<Value>, answers: Vec<Value>) -> SearxngResponse {
        SearxngResponse {
            infoboxes,
            answers,
            ..SearxngResponse::default()
        }
    }

    #[test]
    fn parses_infobox_attributes() {
        let r = resp_with(
            vec![json!({
                "infobox": "Abdullah of Pahang",
                "id": "https://en.wikipedia.org/wiki/Abdullah_of_Pahang",
                "content": "Sultan of Pahang",
                "attributes": [
                    {"label": "Religion", "value": "Sunni Islam"},
                    {"label": "Born", "value": "1959"}
                ]
            })],
            vec![],
        );
        let facts = structured_facts(&r);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].title, "Abdullah of Pahang");
        assert!(facts[0].is_structured_source);
        assert_eq!(facts[0].attributes.len(), 2);
        let md = facts[0].to_markdown();
        assert!(md.contains("Religion: Sunni Islam"));
        assert!(md.contains("Sultan of Pahang"));
    }

    #[test]
    fn parses_string_and_object_answers() {
        let r = resp_with(
            vec![],
            vec![
                json!("42 is the answer"),
                json!({"answer": "Tokyo", "url": "https://x"}),
            ],
        );
        let facts = structured_facts(&r);
        assert_eq!(facts.len(), 2);
        assert_eq!(facts[0].content, "42 is the answer");
        assert_eq!(facts[1].content, "Tokyo");
        assert_eq!(facts[1].url, "https://x");
    }

    #[test]
    fn skips_empty_and_malformed() {
        let r = resp_with(
            vec![
                json!({"infobox": "Empty"}),
                json!({"attributes": []}),
                json!(123),
            ],
            vec![json!(""), json!({"no_answer": "x"}), json!(true)],
        );
        assert_eq!(structured_facts(&r).len(), 0);
    }
}
