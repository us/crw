//! Per-field extraction basis: the honest evidence contract.
//!
//! Everything here is **pure and deterministic** — no network, no LLM. The
//! model proposes an attribution per field; this module is what decides whether
//! to believe it. The rule the whole module exists to enforce:
//!
//! > A field that cannot be verified is marked `unverified` or `unsupported`.
//! > There is no code path that produces a fake attribution.
//!
//! The server never synthesizes a url, a hash, an excerpt or a title. The
//! citation's `url` is the document the server fetched; `source_hash` is the
//! server's own hash of the bytes it sent to the model. The model's `sourceUrl`
//! string is only ever *compared*, then discarded.
//!
//! See `docs/adr/0001-basis-wire-format.md` for the wire format, the decisions
//! behind it, and the honest limits of what these checks can prove.

use crw_core::evidence::{
    BASIS_VERSION, Basis, BasisWarning, ConfidenceLevel, EvidenceCitation, FieldStatus,
};
use serde_json::{Value, json};

/// Max excerpt length we ask for, and enforce. The request-side `maxLength` is
/// only a hint (neither provider constrains decoding), so [`align_basis`] also
/// rejects an over-long excerpt — otherwise the token budget below is fiction.
pub(crate) const EXCERPT_MAX_CHARS: usize = 160;

/// Output-token budget for the non-basis part of the response (the `data`
/// object, which shares the same `max_tokens` since basis rides the same tool
/// call). Measured against the ~700-token typical basis-off extract.
const BASE_OUTPUT_TOKENS: usize = 800;

/// Output-token budget per scalar leaf's basis entry: ~15 keys/punctuation,
/// ~20 for the echoed `sourceUrl`, ~45 for a 160-char excerpt, ~10 for the
/// value and confidence.
const PER_LEAF_OUTPUT_TOKENS: usize = 90;

/// The `sourceTextKind` every basis citation carries: the truncated markdown
/// actually sent to the model, NOT the full normalized markdown.
pub(crate) const SOURCE_TEXT_KIND: &str = "llmInput";

/// Warning codes. Closed, crw-owned set — never upstream text (a basis rides on
/// responses that get persisted and read back by customers).
mod code {
    pub const BASIS_MISSING: &str = "basis_missing";
    pub const BASIS_SOURCE_UNKNOWN: &str = "basis_source_unknown";
    pub const BASIS_VALUE_MISMATCH: &str = "basis_value_mismatch";
    pub const EXCERPT_EMPTY: &str = "excerpt_empty";
    pub const EXCERPT_TOO_LONG: &str = "excerpt_too_long";
    pub const EXCERPT_NOT_IN_SOURCE: &str = "excerpt_not_in_source";
    pub const EXCERPT_MISSING_VALUE: &str = "excerpt_missing_value";
}

/// Is this schema property a scalar we emit basis for? Evidence is emitted only
/// for **top-level scalar** properties; objects and arrays are skipped (their
/// semantics are out of scope in v1).
///
/// Accepts `"type": "string"`, `"type": ["string", "null"]`, and the
/// `anyOf`/`oneOf` nullable-scalar form `pydantic.model_json_schema()` emits for
/// `Optional[str]`: `{"anyOf": [{"type": "string"}, {"type": "null"}]}`. A
/// member with no `type` (e.g. a `$ref`) makes the whole property non-scalar, so
/// a `string | SomeObject` union is not mistaken for a scalar.
fn is_scalar_schema(prop: &Value) -> bool {
    fn scalar_name(t: &str) -> bool {
        matches!(t, "string" | "number" | "integer" | "boolean")
    }
    /// `None` == the node has no `type` key (unknown shape). `Some(list)` == its
    /// declared type name(s).
    fn type_names(node: &Value) -> Option<Vec<String>> {
        match node.get("type") {
            Some(Value::String(t)) => Some(vec![t.clone()]),
            Some(Value::Array(ts)) => Some(
                ts.iter()
                    .filter_map(Value::as_str)
                    .map(String::from)
                    .collect(),
            ),
            None => None,
            _ => Some(vec![]),
        }
    }
    let mut names: Vec<String> = type_names(prop).unwrap_or_default();
    for key in ["anyOf", "oneOf"] {
        if let Some(Value::Array(members)) = prop.get(key) {
            for m in members {
                match type_names(m) {
                    Some(ts) => names.extend(ts),
                    None => return false,
                }
            }
        }
    }
    !names.is_empty()
        && names.iter().all(|t| scalar_name(t) || t == "null")
        && names.iter().any(|t| scalar_name(t))
}

/// The top-level scalar property names of a caller's schema, in schema order.
pub(crate) fn scalar_leaves(schema: &Value) -> Vec<String> {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|props| {
            props
                .iter()
                .filter(|(_, p)| is_scalar_schema(p))
                .map(|(k, _)| k.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Why a schema cannot carry basis. Returned as a plain message the caller
/// wraps in `CrwError::InvalidRequest`.
pub(crate) fn reject_reason(schema: &Value, max_tokens: u32) -> Option<String> {
    if schema.get("type").and_then(Value::as_str) != Some("object") {
        return Some(
            "basis requires a 'jsonSchema' of type 'object' with top-level properties".into(),
        );
    }
    // We inject a `basis` property into the caller's own schema root (so their
    // `$ref`/`$defs` keep resolving); a caller property of that name collides.
    if schema
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|p| p.contains_key("basis"))
    {
        return Some(
            "basis is not supported for a schema with a top-level 'basis' property (name collision)"
                .into(),
        );
    }
    let leaves = scalar_leaves(schema).len();
    if leaves == 0 {
        return Some(
            "basis requires at least one top-level scalar property (string, number, integer or \
             boolean) in the 'jsonSchema'; evidence is not emitted for objects or arrays"
                .into(),
        );
    }
    // Preflight: refuse a schema whose basis cannot fit the model's per-leg
    // output cap. Failing here is free; failing after the call costs the full
    // input-token spend AND lands as an opaque truncated-JSON parse error.
    let needed = BASE_OUTPUT_TOKENS + leaves * PER_LEAF_OUTPUT_TOKENS;
    if needed > max_tokens as usize {
        return Some(format!(
            "basis_schema_too_large: {leaves} top-level scalar properties need about {needed} \
             output tokens, but the configured per-call limit is {max_tokens}. Reduce the \
             schema's scalar properties or raise extraction.llm.max_tokens."
        ));
    }
    None
}

/// The tool schema for a basis extraction: the caller's schema, **unmodified at
/// its root**, with one `basis` property added.
///
/// Deliberately NOT a wrapper around `{ data, basis }`. A caller schema from
/// `pydantic.model_json_schema()` carries `"$ref": "#/$defs/Foo"`, which
/// resolves against the *document root*; nesting it one level down breaks every
/// such ref and the provider rejects the tool. Keeping the caller's schema as
/// the root means `#/$defs/...`, `#/definitions/...` and `#/properties/...` all
/// keep resolving with no rewriting at all.
pub(crate) fn tool_schema(schema: &Value, leaves: &[String]) -> Value {
    let leaf_schema = json!({
        "type": "object",
        "properties": {
            "value": {},
            "sourceUrl": { "type": "string" },
            "excerpt": { "type": ["string", "null"], "maxLength": EXCERPT_MAX_CHARS },
            "confidence": { "enum": ["low", "medium", "high"] },
        },
        // `confidence` is deliberately not required: models omit it constantly,
        // and a strict requirement would fail the whole basis object over it.
        "required": ["value", "sourceUrl", "excerpt"],
    });

    let mut out = schema.clone();
    let obj = out.as_object_mut().expect("checked by reject_reason");

    let mut basis_props = serde_json::Map::new();
    for leaf in leaves {
        basis_props.insert(leaf.clone(), leaf_schema.clone());
    }
    let basis_prop = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": Value::Object(basis_props),
    });

    obj.entry("properties")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("checked by reject_reason")
        .insert("basis".into(), basis_prop);

    // `required` is absent whenever every caller field is optional (pydantic
    // omits it). Without creating it, `basis` would be an *optional* output,
    // the model would skip it, and every leaf would silently degrade to
    // `basis_missing` with no error anywhere.
    if let Some(req) = obj
        .entry("required")
        .or_insert_with(|| json!([]))
        .as_array_mut()
    {
        req.push(json!("basis"));
    }
    out
}

/// The instruction appended to the extraction prompt in basis mode.
pub(crate) fn prompt_section(source_url: &str) -> String {
    format!(
        "\n\n## Source URL\n{source_url}\n\n\
         ## Evidence\n\
         Also call the tool with a `basis` object. For every top-level scalar field you \
         extracted, add an entry keyed by the field name with:\n\
         - `value`: the exact same value you put in that field.\n\
         - `sourceUrl`: the Source URL above, verbatim.\n\
         - `excerpt`: the SHORTEST verbatim span copied from the Content below that contains \
         the value ({EXCERPT_MAX_CHARS} characters or fewer). Copy it character for character; \
         do not paraphrase, summarize or reformat it.\n\
         - `confidence`: low, medium or high.\n\
         If a value is not grounded in a specific span of the Content, set `excerpt` to null. \
         Never invent an excerpt."
    )
}

// ── Deterministic verification ────────────────────────────────────────────

/// Collapse whitespace runs to a single space and trim. Applied to both sides
/// of a containment test so a model that reflows `**Price:** $19.99` into
/// `Price: $19.99` is not punished for it. Losing markdown syntax still fails
/// the check, which is the safe direction.
fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn collapse_ws_lower(s: &str) -> String {
    collapse_ws(&s.to_lowercase())
}

/// Compare a url for identity, not for byte equality: lowercase the scheme and
/// host, drop the fragment, trim one trailing slash. Any stricter and a
/// harmless echo difference becomes a false `unsupported`.
fn norm_url(u: &str) -> String {
    let u = u.trim();
    let u = u.split('#').next().unwrap_or(u);
    let lowered = match u.find("://") {
        Some(i) => {
            let (head, rest) = u.split_at(i + 3);
            let end = rest.find('/').unwrap_or(rest.len());
            let (host, path) = rest.split_at(end);
            format!("{}{}{}", head.to_lowercase(), host.to_lowercase(), path)
        }
        None => u.to_lowercase(),
    };
    lowered.trim_end_matches('/').to_string()
}

/// Strict value equality, with the two traps `serde_json`'s `PartialEq` sets:
/// `json!(20) != json!(20.0)` (different `Number` variants), and `as_f64()`
/// silently equating integers above 2^53. Integers compare as integers; only a
/// float on either side falls back to `f64`.
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            if let (Some(xi), Some(yi)) = (x.as_i64(), y.as_i64()) {
                return xi == yi;
            }
            if let (Some(xu), Some(yu)) = (x.as_u64(), y.as_u64()) {
                return xu == yu;
            }
            match (x.as_f64(), y.as_f64()) {
                (Some(xf), Some(yf)) => xf == yf,
                _ => x == y,
            }
        }
        _ => a == b,
    }
}

/// Render a scalar as the text we expect to find in the source.
fn render_value(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => {
            // 20.0 is written "20" on a page, not "20.0".
            if let Some(f) = n.as_f64()
                && n.as_i64().is_none()
                && n.as_u64().is_none()
                && f.fract() == 0.0
                && f.abs() < 9e15
            {
                return Some(format!("{}", f as i64));
            }
            Some(n.to_string())
        }
        _ => None,
    }
}

/// Strip thousands separators so `1,000,000` on the page grounds `1000000` in
/// the data. Only digit-adjacent separators go, so ordinary prose is untouched.
fn strip_digit_separators(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, &c) in chars.iter().enumerate() {
        let sep = c == ',' || c == '_' || c == ' ' || c == '\u{202f}' || c == '\u{00a0}';
        if sep {
            let prev_digit = i > 0 && chars[i - 1].is_ascii_digit();
            let next_digit = chars.get(i + 1).is_some_and(char::is_ascii_digit);
            if prev_digit && next_digit {
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// Does `needle` occur in `hay` on word boundaries? Bare substring matching is
/// far too weak: `5` occurs in `"Item 5 of 20"` and in `"2025"`, and the string
/// `"IT"` occurs in `"the kit list"`, so any excerpt with a stray token would
/// "ground" the value. `dot_joins` keeps `.` inside the token for numbers
/// (`19.99` must not split at the decimal); for strings `.` is a boundary.
fn contains_word(hay: &str, needle: &str, dot_joins: bool) -> bool {
    if needle.is_empty() {
        return false;
    }
    let hb = hay.as_bytes();
    let nb = needle.as_bytes();
    let boundary =
        |b: Option<&u8>| b.is_none_or(|c| !c.is_ascii_alphanumeric() && !(dot_joins && *c == b'.'));
    let mut from = 0usize;
    while let Some(rel) = hay[from..].find(needle) {
        let start = from + rel;
        let end = start + nb.len();
        if boundary(start.checked_sub(1).map(|i| &hb[i])) && boundary(hb.get(end)) {
            return true;
        }
        // Retry past this match, advancing one CHAR: `start + 1` lands inside the
        // match's first char whenever it is multibyte, and the next iteration
        // slices `hay[from..]` there, which panics. Both sides are page text, so
        // a value like "日本語" embedded in a larger token ("x日本語y") reaches
        // this on an ordinary extract.
        from = start + hay[start..].chars().next().map_or(1, char::len_utf8);
        if from >= hay.len() {
            break;
        }
    }
    false
}

/// Numbers: `.` is part of the token so `19.99` is matched whole.
fn contains_bounded(hay: &str, needle: &str) -> bool {
    contains_word(hay, needle, true)
}

/// Does the excerpt actually **carry** the value?
///
/// This is the check that stops a real-url / real-hash / real-but-irrelevant
/// excerpt from being sold as `supported`. Plain substring containment (which
/// is all the excerpt-in-source check does) is trivially satisfied by
/// `excerpt: "a"` — a substring of every document — so without this, a fully
/// fabricated attribution passes and the honesty gate is true by construction.
///
/// Type-dispatched, because the naive form bars whole types from ever being
/// `supported`:
///
/// - **Empty string**: never carried. `""` is a substring of every excerpt, and
///   `""` is not `null` so it does not qualify as `notFound` — the one input
///   that would otherwise get `supported` for free.
/// - **String**: the excerpt must contain it. A value longer than the excerpt
///   cap physically cannot fit, so for those *only*, the value being verbatim in
///   the document is accepted instead — a strictly stronger grounding claim. The
///   fallback is gated on length: if a short value could have been quoted and
///   wasn't, the excerpt is not evidence for it.
/// - **Number**: separator-stripped, on non-alphanumeric boundaries.
/// - **Bool**: exempt. `true` as a literal token essentially never appears in
///   prose ("In stock" grounds `true`), so demanding it be quoted would make
///   `supported` unreachable for every boolean. It still must pass the document,
///   value-equality and excerpt-in-source checks. This is a disclosed limit.
fn value_carried(value: &Value, excerpt: &str, source: &str) -> bool {
    match value {
        Value::Bool(_) => true,
        Value::String(s) if s.trim().is_empty() => false,
        Value::String(s) => {
            // Word-boundary match, else a 2-char value like "IT" grounds on the
            // "kit" in an unrelated excerpt. `.` is a boundary for strings.
            let needle = collapse_ws_lower(s);
            if contains_word(&collapse_ws_lower(excerpt), &needle, false) {
                return true;
            }
            // A value longer than the excerpt cap physically cannot be quoted in
            // full; whole-source containment is a strictly stronger claim and is
            // specific enough that plain containment is safe.
            s.chars().count() > EXCERPT_MAX_CHARS && collapse_ws_lower(source).contains(&needle)
        }
        Value::Number(_) => {
            let Some(rendered) = render_value(value) else {
                return false;
            };
            let hay = strip_digit_separators(&collapse_ws_lower(excerpt));
            contains_bounded(&hay, &rendered)
        }
        _ => false,
    }
}

/// One model-proposed basis entry, parsed leniently. A malformed entry
/// downgrades its own field and nothing else.
struct Claim {
    value: Option<Value>,
    source_url: Option<String>,
    excerpt: Option<String>,
    confidence: Option<ConfidenceLevel>,
}

fn parse_claim(v: &Value) -> Claim {
    let confidence = v.get("confidence").and_then(Value::as_str).and_then(|c| {
        match c.trim().to_lowercase().as_str() {
            "low" => Some(ConfidenceLevel::Low),
            "medium" => Some(ConfidenceLevel::Medium),
            "high" => Some(ConfidenceLevel::High),
            _ => None,
        }
    });
    Claim {
        value: v.get("value").cloned(),
        source_url: v
            .get("sourceUrl")
            .and_then(Value::as_str)
            .map(str::to_string),
        excerpt: v.get("excerpt").and_then(Value::as_str).map(str::to_string),
        confidence,
    }
}

/// Verify the model's proposed attributions against what the server actually
/// fetched and sent. Returns one [`Basis`] per top-level scalar schema property,
/// plus the coded warnings explaining every downgrade.
///
/// `data` is the **schema-validated** response body and is authoritative: it is
/// never rewritten to match what the model claimed in its basis. `source_text`
/// is the canonical text — the exact bytes sent to the model — and `source_hash`
/// is the server's hash of those bytes.
pub(crate) fn align_basis(
    schema: &Value,
    data: &Value,
    model_basis: Option<&Value>,
    source_url: &str,
    source_hash: &str,
    source_text: &str,
) -> (Vec<Basis>, Vec<BasisWarning>) {
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    let empty = serde_json::Map::new();
    let claims = model_basis.and_then(Value::as_object).unwrap_or(&empty);

    let mut warn = |field: &str, code: &str| {
        warnings.push(BasisWarning {
            field: field.to_string(),
            code: code.to_string(),
        });
    };

    for field in scalar_leaves(schema) {
        let cite = |excerpt: Option<String>| EvidenceCitation {
            url: source_url.to_string(),
            title: None,
            excerpt,
            source_hash: source_hash.to_string(),
            source_text_kind: SOURCE_TEXT_KIND.to_string(),
            char_start: None,
            char_end: None,
        };
        let emit = |value: Option<Value>,
                    status: FieldStatus,
                    confidence: Option<ConfidenceLevel>,
                    citations: Vec<EvidenceCitation>| Basis {
            basis_version: BASIS_VERSION,
            field: field.clone(),
            value,
            status,
            confidence,
            reasoning: None,
            citations,
        };

        // `null` and *absent* are notFound. `0` and `false` are values.
        let Some(actual) = data.get(&field).filter(|v| !v.is_null()) else {
            out.push(emit(None, FieldStatus::NotFound, None, vec![]));
            continue;
        };

        let Some(claim) = claims.get(&field).map(parse_claim) else {
            warn(&field, code::BASIS_MISSING);
            out.push(emit(
                Some(actual.clone()),
                FieldStatus::Unsupported,
                None,
                vec![],
            ));
            continue;
        };
        let conf = claim.confidence;
        let unsupported = |w: &'static str| (Some(actual.clone()), FieldStatus::Unsupported, w);

        // Document check: the model must name the document the server fetched.
        let url_ok = claim
            .source_url
            .as_deref()
            .is_some_and(|u| norm_url(u) == norm_url(source_url));
        if !url_ok {
            let (v, s, w) = unsupported(code::BASIS_SOURCE_UNKNOWN);
            warn(&field, w);
            // Attribution dropped: don't carry the model's confidence onto an
            // untrusted value (matches the basis_missing branch above).
            out.push(emit(v, s, None, vec![]));
            continue;
        }

        // Value check: the model contradicting itself forfeits its attribution.
        // `data` (schema-validated) wins; it is never rewritten to match.
        if !claim
            .value
            .as_ref()
            .is_some_and(|v| values_equal(v, actual))
        {
            let (v, s, w) = unsupported(code::BASIS_VALUE_MISMATCH);
            warn(&field, w);
            // Attribution dropped: don't carry the model's confidence onto an
            // untrusted value (matches the basis_missing branch above).
            out.push(emit(v, s, None, vec![]));
            continue;
        }

        // Excerpt checks. Every failure keeps the citation (the document IS
        // attributable) but drops the excerpt: unverified, not unsupported.
        let unverified = |w: Option<&'static str>| (FieldStatus::Unverified, w);
        let (status, warning) = match claim.excerpt.as_deref().map(str::trim) {
            None => unverified(None),
            Some("") => unverified(Some(code::EXCERPT_EMPTY)),
            Some(e) if e.chars().count() > EXCERPT_MAX_CHARS => {
                unverified(Some(code::EXCERPT_TOO_LONG))
            }
            Some(e) if !collapse_ws(source_text).contains(&collapse_ws(e)) => {
                unverified(Some(code::EXCERPT_NOT_IN_SOURCE))
            }
            Some(e) if !value_carried(actual, e, source_text) => {
                unverified(Some(code::EXCERPT_MISSING_VALUE))
            }
            Some(e) => {
                out.push(emit(
                    Some(actual.clone()),
                    FieldStatus::Supported,
                    conf,
                    vec![cite(Some(e.to_string()))],
                ));
                continue;
            }
        };
        if let Some(w) = warning {
            warn(&field, w);
        }
        out.push(emit(Some(actual.clone()), status, conf, vec![cite(None)]));
    }

    (out, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://example.com/p";
    const HASH: &str = "sha256:deadbeef";
    const SRC: &str = "Widget Pro. **Price:** $19.99 in stock. Reviews: 5 of 20.";

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "price": { "type": "number" },
                "name": { "type": "string" },
            },
        })
    }

    fn claim(value: Value, excerpt: Option<&str>) -> Value {
        json!({
            "value": value,
            "sourceUrl": URL,
            "excerpt": excerpt,
            "confidence": "high",
        })
    }

    fn run(
        schema: &Value,
        data: Value,
        basis: Value,
        src: &str,
    ) -> (Vec<Basis>, Vec<BasisWarning>) {
        let (b, w) = align_basis(schema, &data, Some(&basis), URL, HASH, src);
        for basis in &b {
            assert_invariants(basis, src);
        }
        (b, w)
    }

    /// Charter 5.2, asserted as a property on every Basis every test produces.
    /// A `supported` leaf that does not hold up here is a hard test failure —
    /// which is the whole point of the contract.
    fn assert_invariants(b: &Basis, source_text: &str) {
        let empty = b.citations.is_empty();
        assert_eq!(
            empty,
            matches!(b.status, FieldStatus::Unsupported | FieldStatus::NotFound),
            "citations empty iff unsupported|notFound: {b:?}"
        );
        assert!(b.citations.len() <= 1, "v1 emits 0 or 1 citations: {b:?}");
        assert_eq!(
            b.value.is_none(),
            b.status == FieldStatus::NotFound,
            "value is null iff notFound: {b:?}"
        );
        if let Some(c) = b.citations.first() {
            assert_eq!(c.source_hash, HASH, "hash must be the server's: {b:?}");
            assert_eq!(c.url, URL, "url must be the server's: {b:?}");
            assert_eq!(c.source_text_kind, SOURCE_TEXT_KIND);
            assert!(c.title.is_none(), "v1 never stamps a title");
            assert!(
                c.char_start.is_none() && c.char_end.is_none(),
                "v1: no offsets"
            );
            if c.excerpt.is_some() {
                assert_eq!(
                    b.status,
                    FieldStatus::Supported,
                    "an excerpt is present only on supported: {b:?}"
                );
            }
        }
        if b.status == FieldStatus::Supported {
            let c = b.citations.first().expect("supported implies a citation");
            let e = c.excerpt.as_deref().expect("supported implies an excerpt");
            assert!(
                collapse_ws(source_text).contains(&collapse_ws(e)),
                "supported excerpt must be in the canonical source: {b:?}"
            );
            let v = b.value.as_ref().expect("supported implies a value");
            assert!(
                value_carried(v, e, source_text),
                "supported excerpt must carry the value: {b:?}"
            );
        }
    }

    #[test]
    fn supported_happy_path() {
        let (b, w) = run(
            &schema(),
            json!({ "price": 19.99, "name": "Widget Pro" }),
            json!({
                "price": claim(json!(19.99), Some("Price:** $19.99 in stock")),
                "name": claim(json!("Widget Pro"), Some("Widget Pro. **Price:**")),
            }),
            SRC,
        );
        assert!(w.is_empty(), "expected no warnings, got {w:?}");
        assert!(b.iter().all(|x| x.status == FieldStatus::Supported));
        assert_eq!(b[0].citations[0].source_hash, HASH);
        assert_eq!(b[0].confidence, Some(ConfidenceLevel::High));
    }

    /// The charter's hard gate: a model that contradicts its own extracted
    /// value does not get to keep an attribution.
    #[test]
    fn basis_value_mismatch_is_not_supported() {
        let (b, w) = run(
            &schema(),
            json!({ "price": 19.99, "name": "Widget Pro" }),
            json!({
                "price": claim(json!(5), Some("Price:** $19.99")),
                "name": claim(json!("Widget Pro"), Some("Widget Pro.")),
            }),
            SRC,
        );
        // Look up by field name: serde_json (no preserve_order) sorts object
        // keys, so basis order is alphabetical, not schema/insertion order.
        let price = b.iter().find(|x| x.field == "price").unwrap();
        assert_ne!(price.status, FieldStatus::Supported);
        assert_eq!(price.status, FieldStatus::Unsupported);
        assert!(price.citations.is_empty(), "attribution must be dropped");
        assert_eq!(
            price.value,
            Some(json!(19.99)),
            "data is authoritative and is never rewritten to the model's claim"
        );
        assert!(w.iter().any(|x| x.code == code::BASIS_VALUE_MISMATCH));
    }

    /// The anti-fabrication test. A one-character excerpt is a substring of
    /// every document; without the value-carried check it would be `supported`.
    #[test]
    fn trivial_excerpt_is_not_supported() {
        let (b, w) = run(
            &schema(),
            json!({ "price": 19.99, "name": "Widget Pro" }),
            json!({
                "price": claim(json!(19.99), Some("P")),
                "name": claim(json!("Widget Pro"), Some("P")),
            }),
            SRC,
        );
        assert!(b.iter().all(|x| x.status == FieldStatus::Unverified));
        assert!(b.iter().all(|x| x.citations[0].excerpt.is_none()));
        assert_eq!(
            w.iter()
                .filter(|x| x.code == code::EXCERPT_MISSING_VALUE)
                .count(),
            2
        );
    }

    /// The string source-fallback must not re-open the hole above: a SHORT
    /// value that appears in the document but not in the excerpt is still
    /// unverified. Only a value too long to fit the excerpt cap may fall back.
    #[test]
    fn short_string_in_source_but_not_in_excerpt_is_not_supported() {
        let s = json!({ "type": "object", "properties": { "name": { "type": "string" } } });
        let (b, _) = run(
            &s,
            json!({ "name": "Widget Pro" }),
            json!({ "name": claim(json!("Widget Pro"), Some("Reviews: 5 of 20.")) }),
            SRC,
        );
        assert_eq!(b[0].status, FieldStatus::Unverified);
    }

    /// A short string value must ground on a word boundary: "IT" occurs inside
    /// "kit", so an unrelated but in-source excerpt must not sell it as supported.
    #[test]
    fn short_string_grounded_by_unrelated_word_is_not_supported() {
        let s = json!({ "type": "object", "properties": { "unit": { "type": "string" } } });
        let src = "Ships with the kit list included.";
        let (b, w) = run(
            &s,
            json!({ "unit": "IT" }),
            json!({ "unit": claim(json!("IT"), Some("the kit list")) }),
            src,
        );
        assert_eq!(b[0].status, FieldStatus::Unverified);
        assert!(w.iter().any(|x| x.code == code::EXCERPT_MISSING_VALUE));
    }

    /// pydantic emits `Optional[str]` as `anyOf:[{string},{null}]`; that is a
    /// nullable scalar leaf and must carry basis. A union with a non-scalar
    /// member is skipped.
    #[test]
    fn anyof_nullable_scalar_is_a_leaf() {
        let s = json!({ "type": "object", "properties": {
            "name": { "anyOf": [{ "type": "string" }, { "type": "null" }] },
            "vendor": { "anyOf": [{ "type": "string" }, { "type": "object" }] },
            "ref": { "anyOf": [{ "type": "string" }, { "$ref": "#/$defs/X" }] },
        }});
        assert_eq!(scalar_leaves(&s), vec!["name"]);
        let (b, _) = run(
            &s,
            json!({ "name": "Widget Pro" }),
            json!({ "name": claim(json!("Widget Pro"), Some("Widget Pro. **Price:**")) }),
            SRC,
        );
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].status, FieldStatus::Supported);
    }

    #[test]
    fn long_string_value_falls_back_to_source_containment() {
        let long: String = "a verbatim description sentence. ".repeat(8);
        assert!(long.chars().count() > EXCERPT_MAX_CHARS);
        let src = format!("Intro. {long} Outro.");
        let s = json!({ "type": "object", "properties": { "desc": { "type": "string" } } });
        let (b, w) = run(
            &s,
            json!({ "desc": long }),
            json!({ "desc": claim(json!(long), Some("a verbatim description sentence.")) }),
            &src,
        );
        assert_eq!(b[0].status, FieldStatus::Supported, "warnings: {w:?}");
    }

    #[test]
    fn whitespace_reflow_still_supported() {
        let src = "Item.\n\n**Price:**   $19.99\n";
        let s = json!({ "type": "object", "properties": { "price": { "type": "number" } } });
        let (b, _) = run(
            &s,
            json!({ "price": 19.99 }),
            json!({ "price": claim(json!(19.99), Some("**Price:** $19.99")) }),
            src,
        );
        assert_eq!(b[0].status, FieldStatus::Supported);
    }

    #[test]
    fn numeric_forms_20_and_20_0_are_equal() {
        let s = json!({ "type": "object", "properties": { "qty": { "type": "number" } } });
        let (b, _) = run(
            &s,
            json!({ "qty": 20.0 }),
            json!({ "qty": claim(json!(20), Some("Quantity: 20 units")) }),
            "Quantity: 20 units in stock",
        );
        assert_eq!(b[0].status, FieldStatus::Supported);
    }

    #[test]
    fn large_i64_values_compare_exactly() {
        // Both round to the same f64; an as_f64() compare would call them equal
        // and hand a fabricated value a `supported`.
        assert!(!values_equal(
            &json!(9007199254740993i64),
            &json!(9007199254740992i64)
        ));
    }

    #[test]
    fn thousands_separator_number_is_supported() {
        let src = "Annual revenue: 1,000,000 USD.";
        let s = json!({ "type": "object", "properties": { "revenue": { "type": "integer" } } });
        let (b, _) = run(
            &s,
            json!({ "revenue": 1000000 }),
            json!({ "revenue": claim(json!(1000000), Some("Annual revenue: 1,000,000 USD.")) }),
            src,
        );
        assert_eq!(b[0].status, FieldStatus::Supported);
    }

    /// A bare digit-substring match would ground `5` on "Reviews: 5 of 20" via
    /// the `20` — or on any stray year. Boundaries make the match mean something.
    #[test]
    fn small_number_needs_a_real_boundary_match() {
        assert!(contains_bounded("reviews: 5 of 20.", "5"));
        assert!(!contains_bounded("published in 2025.", "5"));
        assert!(!contains_bounded("version 1.50 shipped", "5"));
    }

    #[test]
    fn boolean_leaf_is_exempt_from_value_carried() {
        let s = json!({ "type": "object", "properties": { "inStock": { "type": "boolean" } } });
        let (b, _) = run(
            &s,
            json!({ "inStock": true }),
            json!({ "inStock": claim(json!(true), Some("$19.99 in stock")) }),
            SRC,
        );
        assert_eq!(b[0].status, FieldStatus::Supported);
    }

    /// `""` is a substring of every excerpt and is not `null`, so without an
    /// explicit rule it would reach `supported` with zero proof.
    #[test]
    fn empty_string_value_is_never_supported() {
        let s = json!({ "type": "object", "properties": { "name": { "type": "string" } } });
        let (b, w) = run(
            &s,
            json!({ "name": "" }),
            json!({ "name": claim(json!(""), Some("Widget Pro.")) }),
            SRC,
        );
        assert_eq!(b[0].status, FieldStatus::Unverified);
        assert!(w.iter().any(|x| x.code == code::EXCERPT_MISSING_VALUE));
    }

    #[test]
    fn excerpt_not_in_source_is_unverified() {
        let s = json!({ "type": "object", "properties": { "price": { "type": "number" } } });
        let (b, w) = run(
            &s,
            json!({ "price": 19.99 }),
            json!({ "price": claim(json!(19.99), Some("Sale price 19.99 today only")) }),
            SRC,
        );
        assert_eq!(b[0].status, FieldStatus::Unverified);
        assert!(b[0].citations[0].excerpt.is_none());
        assert!(w.iter().any(|x| x.code == code::EXCERPT_NOT_IN_SOURCE));
    }

    #[test]
    fn excerpt_empty_and_too_long_are_unverified() {
        let s = json!({ "type": "object", "properties": { "price": { "type": "number" } } });
        let (b, w) = run(
            &s,
            json!({ "price": 19.99 }),
            json!({ "price": claim(json!(19.99), Some("   ")) }),
            SRC,
        );
        assert_eq!(b[0].status, FieldStatus::Unverified);
        assert!(w.iter().any(|x| x.code == code::EXCERPT_EMPTY));

        let long = "x".repeat(EXCERPT_MAX_CHARS + 1);
        let (b, w) = run(
            &s,
            json!({ "price": 19.99 }),
            json!({ "price": claim(json!(19.99), Some(&long)) }),
            SRC,
        );
        assert_eq!(b[0].status, FieldStatus::Unverified);
        assert!(w.iter().any(|x| x.code == code::EXCERPT_TOO_LONG));
    }

    #[test]
    fn null_excerpt_is_unverified_without_a_warning() {
        let s = json!({ "type": "object", "properties": { "price": { "type": "number" } } });
        let (b, w) = run(
            &s,
            json!({ "price": 19.99 }),
            json!({ "price": claim(json!(19.99), None) }),
            SRC,
        );
        assert_eq!(b[0].status, FieldStatus::Unverified);
        assert!(
            w.is_empty(),
            "an honest 'I cannot ground this' is not a warning"
        );
    }

    #[test]
    fn invented_url_is_unsupported() {
        let s = json!({ "type": "object", "properties": { "price": { "type": "number" } } });
        let basis = json!({ "price": {
            "value": 19.99,
            "sourceUrl": "https://evil.example/made-up",
            "excerpt": "Price:** $19.99",
        }});
        let (b, w) = run(&s, json!({ "price": 19.99 }), basis, SRC);
        assert_eq!(b[0].status, FieldStatus::Unsupported);
        assert!(b[0].citations.is_empty());
        assert!(w.iter().any(|x| x.code == code::BASIS_SOURCE_UNKNOWN));
    }

    #[test]
    fn url_normalization_tolerates_harmless_echo_differences() {
        assert_eq!(norm_url("HTTPS://Example.com/p/"), norm_url(URL));
        assert_eq!(norm_url("https://example.com/p#frag"), norm_url(URL));
        assert_ne!(norm_url("https://example.com/other"), norm_url(URL));
    }

    #[test]
    fn missing_basis_entry_is_unsupported() {
        let (b, w) = run(
            &schema(),
            json!({ "price": 19.99, "name": "Widget Pro" }),
            json!({ "price": claim(json!(19.99), Some("Price:** $19.99")) }),
            SRC,
        );
        let name = b.iter().find(|x| x.field == "name").unwrap();
        assert_eq!(name.status, FieldStatus::Unsupported);
        assert!(name.citations.is_empty());
        assert_eq!(name.value, Some(json!("Widget Pro")));
        assert!(w.iter().any(|x| x.code == code::BASIS_MISSING));
    }

    #[test]
    fn absent_basis_object_degrades_every_leaf() {
        let (b, w) = align_basis(
            &schema(),
            &json!({ "price": 19.99, "name": "Widget Pro" }),
            None,
            URL,
            HASH,
            SRC,
        );
        assert_eq!(b.len(), 2);
        assert!(b.iter().all(|x| x.status == FieldStatus::Unsupported));
        assert_eq!(w.len(), 2);
    }

    #[test]
    fn null_value_is_not_found() {
        let (b, w) = run(
            &schema(),
            json!({ "price": null, "name": "Widget Pro" }),
            json!({
                "price": claim(json!(19.99), Some("Price:** $19.99")),
                "name": claim(json!("Widget Pro"), Some("Widget Pro.")),
            }),
            SRC,
        );
        let price = b.iter().find(|x| x.field == "price").unwrap();
        assert_eq!(price.status, FieldStatus::NotFound);
        assert_eq!(price.value, None);
        assert!(price.citations.is_empty());
        assert!(
            !w.iter().any(|x| x.field == "price"),
            "an honestly-absent field is not a warning"
        );
    }

    /// `0` and `false` are values, not absences. A falsy test here would
    /// silently discard every zero price and every false flag.
    #[test]
    fn zero_and_false_are_not_not_found() {
        let s = json!({ "type": "object", "properties": {
            "count": { "type": "integer" }, "active": { "type": "boolean" },
        }});
        let src = "count: 0 items. inactive.";
        let (b, _) = run(
            &s,
            json!({ "count": 0, "active": false }),
            json!({
                "count": claim(json!(0), Some("count: 0 items.")),
                "active": claim(json!(false), Some("inactive.")),
            }),
            src,
        );
        assert!(b.iter().all(|x| x.status == FieldStatus::Supported));
    }

    #[test]
    fn non_scalar_properties_get_no_basis() {
        let s = json!({ "type": "object", "properties": {
            "price": { "type": "number" },
            "tags": { "type": "array", "items": { "type": "string" } },
            "vendor": { "type": "object", "properties": { "n": { "type": "string" } } },
            "nullableName": { "type": ["string", "null"] },
        }});
        assert_eq!(scalar_leaves(&s), vec!["nullableName", "price"]);
        let (b, _) = run(
            &s,
            json!({ "price": 19.99, "tags": ["a"], "vendor": {"n":"x"}, "nullableName": null }),
            json!({ "price": claim(json!(19.99), Some("Price:** $19.99")) }),
            SRC,
        );
        assert_eq!(b.len(), 2, "only the two scalar leaves");
        assert!(b.iter().all(|x| x.field != "tags" && x.field != "vendor"));
    }

    #[test]
    fn basis_entry_for_an_unknown_key_is_ignored() {
        let s = json!({ "type": "object", "properties": { "price": { "type": "number" } } });
        let (b, _) = run(
            &s,
            json!({ "price": 19.99 }),
            json!({
                "price": claim(json!(19.99), Some("Price:** $19.99")),
                "ghost": claim(json!("boo"), Some("Widget Pro.")),
            }),
            SRC,
        );
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].field, "price");
    }

    #[test]
    fn malformed_claim_downgrades_only_its_own_field() {
        let (b, _) = run(
            &schema(),
            json!({ "price": 19.99, "name": "Widget Pro" }),
            json!({
                "price": "not an object",
                "name": claim(json!("Widget Pro"), Some("Widget Pro.")),
            }),
            SRC,
        );
        let price = b.iter().find(|x| x.field == "price").unwrap();
        let name = b.iter().find(|x| x.field == "name").unwrap();
        assert_eq!(price.status, FieldStatus::Unsupported);
        assert_eq!(name.status, FieldStatus::Supported);
    }

    // ── tool schema / preflight ───────────────────────────────────────────

    /// The caller's schema stays the document root, so a pydantic-style
    /// `"$ref": "#/$defs/Foo"` still resolves. Nesting it under a wrapper is
    /// what breaks that, and it fails as a provider 400, not a test.
    #[test]
    fn tool_schema_preserves_the_caller_root_and_refs() {
        let caller = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://example.com/product.json",
            "type": "object",
            "$defs": { "Money": { "type": "number" } },
            "properties": {
                "price": { "$ref": "#/$defs/Money", "type": "number" },
                "name": { "type": "string" },
            },
            "required": ["price"],
        });
        let out = tool_schema(&caller, &scalar_leaves(&caller));
        assert_eq!(out["$defs"], caller["$defs"], "$defs stays at the root");
        assert_eq!(out["$id"], caller["$id"]);
        assert_eq!(out["properties"]["price"]["$ref"], "#/$defs/Money");
        assert!(out["properties"]["basis"]["properties"]["price"].is_object());
        assert!(out["properties"]["basis"]["properties"]["name"].is_object());
        let req: Vec<&str> = out["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(req.contains(&"price") && req.contains(&"basis"));
        // The caller's own schema object is untouched.
        assert!(caller["properties"].get("basis").is_none());
    }

    /// pydantic omits `required` entirely when every field is optional. If we
    /// don't create it, `basis` is optional, the model skips it, and every leaf
    /// silently degrades to `basis_missing` with no error anywhere.
    #[test]
    fn tool_schema_creates_required_when_absent() {
        let caller = json!({
            "type": "object",
            "properties": { "name": { "type": "string" } },
        });
        let out = tool_schema(&caller, &scalar_leaves(&caller));
        assert_eq!(out["required"], json!(["basis"]));
    }

    #[test]
    fn preflight_rejects_what_cannot_carry_basis() {
        let ok = json!({ "type": "object", "properties": { "a": { "type": "string" } } });
        assert!(reject_reason(&ok, 4096).is_none());

        assert!(reject_reason(&json!({ "type": "array" }), 4096).is_some());

        let collide = json!({ "type": "object", "properties": {
            "basis": { "type": "string" }, "a": { "type": "string" },
        }});
        assert!(reject_reason(&collide, 4096).unwrap().contains("collision"));

        let no_scalars = json!({ "type": "object", "properties": {
            "tags": { "type": "array", "items": { "type": "string" } },
        }});
        assert!(reject_reason(&no_scalars, 4096).is_some());
    }

    /// The preflight must admit the 30 top-level properties the product allows,
    /// and refuse a schema whose basis would overrun the model's output cap —
    /// failing free instead of after the full input-token spend.
    #[test]
    fn preflight_admits_30_leaves_at_the_default_cap_and_refuses_a_bloated_one() {
        let leaves = |n: usize| {
            let props: serde_json::Map<String, Value> = (0..n)
                .map(|i| (format!("f{i}"), json!({ "type": "string" })))
                .collect();
            json!({ "type": "object", "properties": props })
        };
        assert!(
            reject_reason(&leaves(30), 4096).is_none(),
            "the 30-property product cap must fit the default max_tokens"
        );
        let too_big = reject_reason(&leaves(80), 4096).unwrap();
        assert!(too_big.contains("basis_schema_too_large"), "{too_big}");
        // A self-hoster with a small cap is told, not silently truncated.
        assert!(
            reject_reason(&leaves(10), 1024)
                .unwrap()
                .contains("basis_schema_too_large")
        );
    }

    #[test]
    fn prompt_section_names_the_document() {
        let p = prompt_section(URL);
        assert!(p.contains(URL), "the model cannot echo a url it never saw");
        assert!(p.contains("basis"));
    }

    /// A value embedded in a larger token fails the word-boundary check, so the
    /// scan retries past the match — and every retry must land on a char
    /// boundary. Both the excerpt and the value are page text, so a multibyte
    /// first char here is ordinary (CJK, accented Latin, emoji), not an edge
    /// case. The whole ladder is exercised: no match, an embedded-only match,
    /// and an embedded match followed by a real standalone one.
    ///
    /// Every value must still START with a multibyte char after
    /// `collapse_ws_lower`, or it never reaches the hazard: Turkish 'İ'
    /// lowercases to ASCII 'i' plus a combining dot, so an "İstanbul" row would
    /// pass whether or not the retry is char-aware. 2-, 3- and 4-byte first
    /// chars are all covered.
    #[test]
    fn a_multibyte_value_embedded_in_a_token_does_not_panic_the_scan() {
        for (value, embedded, standalone) in [
            ("日本語", "x日本語y", "日本語 の"),
            ("Ölçek", "xÖlçekli", "Ölçek değeri"),
            ("😀", "x😀y", "😀 tag"),
        ] {
            let needle = collapse_ws_lower(value);
            // Embedded only: the retry path runs to exhaustion and finds nothing.
            assert!(
                !contains_word(&collapse_ws_lower(embedded), &needle, false),
                "{value} is only embedded in {embedded}, so it is not carried"
            );
            // Embedded first, then standalone: the retry must survive the first
            // match and still reach the second.
            let both = collapse_ws_lower(&format!("{embedded} {standalone}"));
            assert!(
                contains_word(&both, &needle, false),
                "{value} stands alone later in {both} and must be found"
            );
        }
    }

    /// The same retry path, reached through the real grounding entry point.
    ///
    /// The value must still start with a multibyte char *after* `collapse_ws_lower`
    /// to reach the hazard: Turkish 'İ' lowercases to ASCII 'i' plus a combining
    /// dot, so an "İstanbul" fixture here would pass whether or not the retry is
    /// char-aware. '日' lowercases to itself.
    #[test]
    fn value_carried_survives_a_multibyte_value_embedded_in_the_excerpt() {
        // The value is short, so the whole-source fallback never applies and the
        // verdict rests purely on the excerpt scan.
        let source = "a document that happens to mention 日本語 somewhere";
        assert!(!value_carried(
            &json!("日本語"),
            "x日本語y is not the language",
            source
        ));
        assert!(value_carried(
            &json!("日本語"),
            "x日本語y, then 日本語 itself",
            source
        ));
    }
}
