//! Structured tool response envelope. Each tool returns `ToolResponse<T>` with
//! the per-tool payload under `data`. Serializes to JSON embedded as MCP text
//! content.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ToolResponse<T: Serialize> {
    pub ok: bool,
    pub session: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub navigated: bool,
    pub elapsed_ms: u64,
    pub data: T,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<String>>,
}

impl<T: Serialize> ToolResponse<T> {
    pub fn new(session: impl Into<String>, url: Option<String>, data: T) -> Self {
        Self {
            ok: true,
            session: session.into(),
            url,
            title: None,
            navigated: false,
            elapsed_ms: 0,
            data,
            warnings: None,
        }
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn with_navigated(mut self, navigated: bool) -> Self {
        self.navigated = navigated;
        self
    }

    pub fn with_elapsed_ms(mut self, elapsed_ms: u64) -> Self {
        self.elapsed_ms = elapsed_ms;
        self
    }

    pub fn with_warning(mut self, msg: impl Into<String>) -> Self {
        self.warnings.get_or_insert_with(Vec::new).push(msg.into());
        self
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| String::from("{}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Serialize)]
    struct Dummy {
        value: u32,
    }

    #[test]
    fn serializes_minimal_response() {
        let r = ToolResponse::new("s1", Some("https://example.com".into()), Dummy { value: 7 });
        let json: serde_json::Value = serde_json::from_str(&r.to_json()).unwrap();
        assert_eq!(json["ok"], true);
        assert_eq!(json["session"], "s1");
        assert_eq!(json["url"], "https://example.com");
        assert_eq!(json["data"]["value"], 7);
        assert!(!json.as_object().unwrap().contains_key("warnings"));
        assert!(
            !json.as_object().unwrap().contains_key("title"),
            "title skipped when unset"
        );
    }

    #[test]
    fn url_skipped_when_none() {
        let r = ToolResponse::new("s1", None, Dummy { value: 0 });
        let json: serde_json::Value = serde_json::from_str(&r.to_json()).unwrap();
        assert!(
            !json.as_object().unwrap().contains_key("url"),
            "url skipped when None"
        );
    }

    #[test]
    fn builder_methods_chain() {
        let r = ToolResponse::new("s2", Some("https://x.y".into()), Dummy { value: 1 })
            .with_title("Hello")
            .with_navigated(true)
            .with_elapsed_ms(123)
            .with_warning("status unknown");
        let json: serde_json::Value = serde_json::from_str(&r.to_json()).unwrap();
        assert_eq!(json["title"], "Hello");
        assert_eq!(json["navigated"], true);
        assert_eq!(json["elapsed_ms"], 123);
        assert_eq!(json["warnings"][0], "status unknown");
    }
}
