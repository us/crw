use regex::Regex;
use scraper::{Html, Selector};
use std::sync::OnceLock;

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
pub fn extract_by_css(html: &str, selector: &str) -> Option<String> {
    let sel = Selector::parse(selector).ok()?;
    let doc = Html::parse_document(html);
    let matched: Vec<String> = doc.select(&sel).map(|el| el.html()).collect();
    if matched.is_empty() {
        None
    } else {
        Some(matched.join("\n"))
    }
}

/// Extract text content matching an XPath expression.
/// Returns the string value of the XPath result.
/// Script and style elements are removed first to prevent their content
/// from leaking into text nodes.
pub fn extract_by_xpath(html: &str, xpath_expr: &str) -> Option<String> {
    let clean = strip_noise(html);
    let package = sxd_html::parse_html(&clean);
    let document = package.as_document();

    let factory = sxd_xpath::Factory::new();
    let xpath = match factory.build(xpath_expr) {
        Ok(Some(xp)) => xp,
        _ => return None,
    };

    let context = sxd_xpath::Context::new();
    let value = xpath.evaluate(&context, document.root()).ok()?;
    let s = value.string();
    if s.is_empty() { None } else { Some(s) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_extracts_article() {
        let html = "<html><body><nav>Nav</nav><article><p>Main</p></article></body></html>";
        let result = extract_by_css(html, "article").unwrap();
        assert!(result.contains("Main"));
        assert!(!result.contains("Nav"));
    }

    #[test]
    fn css_returns_none_for_missing_selector() {
        let html = "<html><body><p>Hello</p></body></html>";
        assert!(extract_by_css(html, "article").is_none());
    }

    #[test]
    fn css_multiple_matches_joined() {
        let html = "<html><body><p class='item'>A</p><p class='item'>B</p></body></html>";
        let result = extract_by_css(html, ".item").unwrap();
        assert!(result.contains('A'));
        assert!(result.contains('B'));
    }

    #[test]
    fn xpath_extracts_text() {
        let html = "<html><body><h1>Title</h1><p>Content</p></body></html>";
        let result = extract_by_xpath(html, "//h1");
        assert!(result.is_some());
        assert!(result.unwrap().contains("Title"));
    }

    #[test]
    fn xpath_returns_none_for_no_match() {
        let html = "<html><body><p>Hello</p></body></html>";
        let result = extract_by_xpath(html, "//article");
        assert!(result.is_none());
    }
}
