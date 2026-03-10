use regex::Regex;
use scraper::{Html, Selector};
use std::sync::OnceLock;
use sxd_xpath::Value;

static RE_SCRIPT: OnceLock<Regex> = OnceLock::new();
static RE_STYLE: OnceLock<Regex> = OnceLock::new();

/// Strip <script> and <style> elements so their content does not pollute
/// text extraction in XPath string values.
fn strip_noise(html: &str) -> String {
    let rs = RE_SCRIPT.get_or_init(|| Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap());
    let ry = RE_STYLE.get_or_init(|| Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap());
    let s = rs.replace_all(html, "");
    ry.replace_all(&s, "").into_owned()
}

/// Extract HTML content matching a CSS selector.
/// Returns the concatenated outer HTML of all matched elements.
pub fn extract_by_css(html: &str, selector: &str) -> Result<Option<String>, String> {
    let sel = Selector::parse(selector)
        .map_err(|err| format!("Invalid CSS selector '{selector}': {err}"))?;
    let doc = Html::parse_document(html);
    let matched: Vec<String> = doc.select(&sel).map(|el| el.html()).collect();
    if matched.is_empty() {
        Ok(None)
    } else {
        Ok(Some(matched.join("\n")))
    }
}

/// Extract text content matching an XPath expression.
/// Returns the string value of each XPath match in document order.
/// Script and style elements are removed first to prevent their content
/// from leaking into text nodes.
pub fn extract_by_xpath(html: &str, xpath_expr: &str) -> Result<Option<Vec<String>>, String> {
    let clean = strip_noise(html);
    let package = sxd_html::parse_html(&clean);
    let document = package.as_document();

    let factory = sxd_xpath::Factory::new();
    let xpath = match factory.build(xpath_expr) {
        Ok(Some(xp)) => xp,
        Ok(None) => return Ok(None),
        Err(err) => return Err(format!("Invalid XPath selector '{xpath_expr}': {err}")),
    };

    let context = sxd_xpath::Context::new();
    let value = xpath
        .evaluate(&context, document.root())
        .map_err(|err| format!("XPath evaluation failed for '{xpath_expr}': {err}"))?;

    let matches = match value {
        Value::Nodeset(nodeset) => nodeset
            .document_order()
            .into_iter()
            .map(|node| node.string_value().trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>(),
        Value::String(value) => {
            let value = value.trim().to_string();
            if value.is_empty() {
                Vec::new()
            } else {
                vec![value]
            }
        }
        Value::Number(value) => vec![value.to_string()],
        Value::Boolean(value) => vec![value.to_string()],
    };

    if matches.is_empty() {
        Ok(None)
    } else {
        Ok(Some(matches))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_extracts_article() {
        let html = "<html><body><nav>Nav</nav><article><p>Main</p></article></body></html>";
        let result = extract_by_css(html, "article").unwrap().unwrap();
        assert!(result.contains("Main"));
        assert!(!result.contains("Nav"));
    }

    #[test]
    fn css_returns_none_for_missing_selector() {
        let html = "<html><body><p>Hello</p></body></html>";
        assert!(extract_by_css(html, "article").unwrap().is_none());
    }

    #[test]
    fn css_multiple_matches_joined() {
        let html = "<html><body><p class='item'>A</p><p class='item'>B</p></body></html>";
        let result = extract_by_css(html, ".item").unwrap().unwrap();
        assert!(result.contains('A'));
        assert!(result.contains('B'));
    }

    #[test]
    fn xpath_extracts_all_matches() {
        let html = "<html><body><h1>Title</h1><p>Content</p></body></html>";
        let values = extract_by_xpath(html, "//*").unwrap().unwrap();
        assert!(values.iter().any(|value| value.contains("Title")));
        assert!(values.iter().any(|value| value.contains("Content")));
    }

    #[test]
    fn xpath_returns_none_for_no_match() {
        let html = "<html><body><p>Hello</p></body></html>";
        let result = extract_by_xpath(html, "//article").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn invalid_css_returns_error() {
        let html = "<html><body><p>Hello</p></body></html>";
        let err = extract_by_css(html, "[").unwrap_err();
        assert!(err.contains("Invalid CSS selector"));
    }

    #[test]
    fn invalid_xpath_returns_error() {
        let html = "<html><body><p>Hello</p></body></html>";
        let err = extract_by_xpath(html, "//*[").expect_err("invalid xpath should return an error");
        assert!(err.contains("Invalid XPath selector"));
    }
}
