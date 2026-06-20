//! v2 `formats` model.
//!
//! Firecrawl v2 changed `formats` from an array of strings (v1) to an array of
//! objects (`{"type":"json","schema":...}`) while still accepting bare strings
//! that auto-coerce to `{type}`. This module parses that union with a
//! hand-rolled `Deserialize` (NOT `#[serde(untagged)]`, which collapses all
//! errors into an opaque "did not match any variant") and `decompose`s it into
//! the exact internal shape v1 already uses — `Vec<OutputFormat>` plus the
//! sibling option fields (`json_schema`, `change_tracking`) — so the v2 handlers
//! reuse `crw_crawl::single::scrape_url` unchanged.

use serde::Deserialize;
use serde::de::{self, Deserializer};
use serde_json::{Map, Value};

use crw_core::types::{ChangeTrackingMode, ChangeTrackingOptions, OutputFormat};

/// One entry of a v2 `formats` array: a bare string (`"markdown"`) or an object
/// (`{"type":"json","schema":...}`).
#[derive(Debug, Clone)]
pub enum FormatSpec {
    String(String),
    Object(Map<String, Value>),
}

impl<'de> Deserialize<'de> for FormatSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match Value::deserialize(deserializer)? {
            Value::String(s) => Ok(FormatSpec::String(s)),
            Value::Object(m) => Ok(FormatSpec::Object(m)),
            other => Err(de::Error::custom(format!(
                "each entry of `formats` must be a string or an object with a `type` field, got {}",
                json_kind(&other)
            ))),
        }
    }
}

fn json_kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// The v2 formats decomposed into the internal v1-style shape.
#[derive(Debug, Default)]
pub struct DecomposedFormats {
    /// Internal formats fed to `ScrapeRequest.formats`.
    pub formats: Vec<OutputFormat>,
    /// `ScrapeRequest.json_schema` (from a `{"type":"json","schema":...}` entry).
    pub json_schema: Option<Value>,
    /// `ScrapeRequest.change_tracking` (from a `{"type":"changeTracking",...}` entry).
    pub change_tracking: Option<ChangeTrackingOptions>,
    /// A `screenshot` format was requested. The format is now produced via CDP
    /// `Page.captureScreenshot`; this flag is retained for callers that want to
    /// branch on it (e.g. credit pricing).
    pub screenshot_requested: bool,
    /// Whether the requested screenshot should be full-page (`screenshot@fullPage`
    /// or `{type:"screenshot", fullPage:true}`) rather than viewport-only.
    /// Copied into `ScrapeRequest.screenshot_full_page` by `to_internal`.
    pub screenshot_full_page: bool,
    /// v2-only formats crw cannot yet produce (`images`, `attributes`,
    /// `branding`, `audio`, `query`). Surfaced as a warning.
    pub unsupported: Vec<String>,
}

/// Fold a v2 `formats` array into the internal shape. Returns a human-readable
/// error (mapped to HTTP 400 by the caller) on an invalid entry.
pub fn decompose(specs: &[FormatSpec]) -> Result<DecomposedFormats, String> {
    let mut out = DecomposedFormats::default();
    let mut screenshot_seen = false;

    for spec in specs {
        match spec {
            FormatSpec::String(s) => handle_token(s, None, &mut out, &mut screenshot_seen)?,
            FormatSpec::Object(m) => {
                let ty = m.get("type").and_then(Value::as_str).ok_or_else(|| {
                    "each `formats` object requires a string `type` field".to_string()
                })?;
                handle_token(ty, Some(m), &mut out, &mut screenshot_seen)?;
            }
        }
    }

    // changeTracking diffs markdown content — ensure markdown is produced even
    // if the caller didn't list it (mirrors Firecrawl's requirement).
    if out.formats.contains(&OutputFormat::ChangeTracking)
        && !out.formats.contains(&OutputFormat::Markdown)
    {
        out.formats.push(OutputFormat::Markdown);
    }

    // Empty after filtering unsupported-only formats → default to markdown so a
    // scrape always returns content (matches v2 default `[{"type":"markdown"}]`).
    if out.formats.is_empty() {
        out.formats.push(OutputFormat::Markdown);
    }

    Ok(out)
}

fn handle_token(
    ty: &str,
    obj: Option<&Map<String, Value>>,
    out: &mut DecomposedFormats,
    screenshot_seen: &mut bool,
) -> Result<(), String> {
    match ty {
        "screenshot" | "screenshot@fullPage" => {
            if *screenshot_seen {
                return Err("only one screenshot format allowed per request".to_string());
            }
            *screenshot_seen = true;
            out.screenshot_requested = true;
            // fullPage from the string suffix (`screenshot@fullPage`) or the
            // object's `fullPage` key. `quality` / `viewport` keys are
            // accepted-and-ignored for drop-in compatibility (D6).
            // ponytail: quality + custom viewport deferred to v2 follow-up.
            let full_page = ty == "screenshot@fullPage"
                || obj
                    .and_then(|m| m.get("fullPage"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
            out.screenshot_full_page = full_page;
            if !out.formats.contains(&OutputFormat::Screenshot) {
                out.formats.push(OutputFormat::Screenshot);
            }
            Ok(())
        }
        "images" | "attributes" | "branding" | "audio" | "query" => {
            out.unsupported.push(ty.to_string());
            Ok(())
        }
        _ => {
            // Everything else routes through the shared v1 token parser so the
            // accepted set + error wording stay identical across versions.
            let fmt = OutputFormat::parse_loose(ty)?;
            if !out.formats.contains(&fmt) {
                out.formats.push(fmt);
            }
            if fmt == OutputFormat::Json
                && let Some(m) = obj
                && let Some(schema) = m.get("schema")
            {
                out.json_schema = Some(schema.clone());
            }
            if fmt == OutputFormat::ChangeTracking {
                out.change_tracking = Some(match obj {
                    Some(m) => parse_change_tracking(m)?,
                    None => ChangeTrackingOptions {
                        modes: vec![ChangeTrackingMode::GitDiff],
                        ..Default::default()
                    },
                });
            }
            Ok(())
        }
    }
}

fn parse_change_tracking(m: &Map<String, Value>) -> Result<ChangeTrackingOptions, String> {
    let modes = match m.get("modes") {
        Some(v) => serde_json::from_value::<Vec<ChangeTrackingMode>>(v.clone())
            .map_err(|e| format!("invalid changeTracking modes: {e}"))?,
        None => vec![ChangeTrackingMode::GitDiff],
    };
    Ok(ChangeTrackingOptions {
        modes: if modes.is_empty() {
            vec![ChangeTrackingMode::GitDiff]
        } else {
            modes
        },
        schema: m.get("schema").cloned(),
        prompt: m.get("prompt").and_then(Value::as_str).map(str::to_string),
        previous: None,
        tag: m.get("tag").and_then(Value::as_str).map(str::to_string),
        content_type: None,
    })
}

/// Join the unsupported-format names into a `warning` string, or `None`.
pub fn unsupported_warning(unsupported: &[String]) -> Option<String> {
    if unsupported.is_empty() {
        return None;
    }
    Some(format!(
        "the following requested formats are not yet produced by this engine and were ignored: {}",
        unsupported.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn specs(v: serde_json::Value) -> Vec<FormatSpec> {
        serde_json::from_value(v).expect("formats parse")
    }

    #[test]
    fn bare_strings_parse() {
        let d = decompose(&specs(json!(["markdown", "html", "links"]))).unwrap();
        assert_eq!(
            d.formats,
            vec![
                OutputFormat::Markdown,
                OutputFormat::Html,
                OutputFormat::Links
            ]
        );
    }

    #[test]
    fn object_json_lifts_schema() {
        let schema = json!({"type": "object", "properties": {"title": {"type": "string"}}});
        let d = decompose(&specs(json!([
            {"type": "json", "schema": schema.clone()},
            {"type": "summary"}
        ])))
        .unwrap();
        assert!(d.formats.contains(&OutputFormat::Json));
        assert!(d.formats.contains(&OutputFormat::Summary));
        assert_eq!(d.json_schema.as_ref(), Some(&schema));
    }

    #[test]
    fn mixed_string_and_object() {
        let d = decompose(&specs(json!([
            "markdown",
            {"type": "json", "schema": {"type": "object"}}
        ])))
        .unwrap();
        assert!(d.formats.contains(&OutputFormat::Markdown));
        assert!(d.formats.contains(&OutputFormat::Json));
    }

    #[test]
    fn two_screenshots_rejected() {
        let err = decompose(&specs(json!([
            {"type": "screenshot"},
            {"type": "screenshot", "fullPage": true}
        ])))
        .unwrap_err();
        assert!(err.contains("only one screenshot"));
    }

    #[test]
    fn change_tracking_auto_adds_markdown() {
        let d = decompose(&specs(
            json!([{"type": "changeTracking", "modes": ["gitDiff"]}]),
        ))
        .unwrap();
        assert!(d.formats.contains(&OutputFormat::Markdown));
        assert!(d.formats.contains(&OutputFormat::ChangeTracking));
        assert!(d.change_tracking.is_some());
    }

    #[test]
    fn unsupported_formats_collected_not_fatal() {
        let d = decompose(&specs(
            json!(["markdown", {"type": "images"}, {"type": "audio"}]),
        ))
        .unwrap();
        assert!(d.formats.contains(&OutputFormat::Markdown));
        assert!(d.unsupported.contains(&"images".to_string()));
        assert!(d.unsupported.contains(&"audio".to_string()));
        assert!(unsupported_warning(&d.unsupported).is_some());
    }

    #[test]
    fn screenshot_string_parses() {
        let d = decompose(&specs(json!(["screenshot"]))).unwrap();
        assert!(d.formats.contains(&OutputFormat::Screenshot));
        assert!(!d.screenshot_full_page);
        assert!(d.screenshot_requested);
        // not surfaced as unsupported anymore — it's produced.
        assert!(!d.unsupported.contains(&"screenshot".to_string()));
    }

    #[test]
    fn screenshot_full_page_string_sets_flag() {
        let d = decompose(&specs(json!(["screenshot@fullPage"]))).unwrap();
        assert!(d.formats.contains(&OutputFormat::Screenshot));
        assert!(d.screenshot_full_page);
    }

    #[test]
    fn screenshot_object_full_page_sets_flag() {
        let d = decompose(&specs(json!([{"type": "screenshot", "fullPage": true}]))).unwrap();
        assert!(d.formats.contains(&OutputFormat::Screenshot));
        assert!(d.screenshot_full_page);
    }

    #[test]
    fn screenshot_object_ignores_quality_and_viewport() {
        // Forgiving over erroring (D6): unknown sub-keys are accepted, not 400'd.
        let d = decompose(&specs(json!([
            {"type": "screenshot", "fullPage": false, "quality": 80,
             "viewport": {"width": 1280, "height": 720}}
        ])))
        .unwrap();
        assert!(d.formats.contains(&OutputFormat::Screenshot));
        assert!(!d.screenshot_full_page);
    }

    #[test]
    fn screenshot_only_does_not_force_markdown() {
        let d = decompose(&specs(json!(["screenshot"]))).unwrap();
        assert_eq!(d.formats, vec![OutputFormat::Screenshot]);
        assert!(!d.formats.contains(&OutputFormat::Markdown));
    }

    #[test]
    fn object_missing_type_errors() {
        let err = decompose(&specs(json!([{"schema": {}}]))).unwrap_err();
        assert!(err.contains("type"));
    }

    #[test]
    fn unknown_format_errors_with_v1_wording() {
        let err = decompose(&specs(json!(["bogus"]))).unwrap_err();
        assert!(err.contains("Unknown format 'bogus'"));
    }

    #[test]
    fn empty_formats_default_to_markdown() {
        let d = decompose(&[]).unwrap();
        assert_eq!(d.formats, vec![OutputFormat::Markdown]);
    }
}
