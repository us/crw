use scraper::{Html, Selector};

/// Extract the "main content" element from HTML using simple heuristics.
/// Returns the inner HTML of the best candidate, or the full body if no candidate found.
pub fn extract_main_content(html: &str) -> String {
    let document = Html::parse_document(html);

    let selectors = [
        "article",
        "main",
        "[role=\"main\"]",
        ".post-content",
        ".article-body",
        ".entry-content",
        "#content",
        ".content",
    ];

    for sel_str in &selectors {
        if let Ok(sel) = Selector::parse(sel_str)
            && let Some(el) = document.select(&sel).next()
        {
            return el.html();
        }
    }

    if let Ok(sel) = Selector::parse("body")
        && let Some(body) = document.select(&sel).next()
    {
        return body.inner_html();
    }

    html.to_string()
}

/// All extracted metadata from a page.
pub struct ExtractedMetadata {
    pub title: Option<String>,
    pub description: Option<String>,
    pub language: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub og_image: Option<String>,
    pub canonical_url: Option<String>,
}

/// Extract metadata (title, description, OG tags, canonical) from HTML.
pub fn extract_metadata(html: &str) -> ExtractedMetadata {
    let document = Html::parse_document(html);

    let title = select_text(&document, "title");

    let description = select_attr(&document, r#"meta[name="description"]"#, "content");

    let og_title = select_attr(&document, r#"meta[property="og:title"]"#, "content");
    let og_description = select_attr(&document, r#"meta[property="og:description"]"#, "content");
    let og_image = select_attr(&document, r#"meta[property="og:image"]"#, "content");

    let canonical_url = select_attr(&document, r#"link[rel="canonical"]"#, "href");

    // Extract language from <html lang="..."> attribute.
    let language = select_attr(&document, "html", "lang");

    ExtractedMetadata {
        title,
        description,
        language,
        og_title,
        og_description,
        og_image,
        canonical_url,
    }
}

fn select_text(doc: &Html, selector: &str) -> Option<String> {
    Selector::parse(selector)
        .ok()
        .and_then(|sel| doc.select(&sel).next())
        .map(|el| el.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty())
}

fn select_attr(doc: &Html, selector: &str, attr: &str) -> Option<String> {
    Selector::parse(selector)
        .ok()
        .and_then(|sel| doc.select(&sel).next())
        .and_then(|el| el.value().attr(attr).map(|s| s.to_string()))
        .filter(|s| !s.is_empty())
}

/// Extract all links from HTML.
pub fn extract_links(html: &str, base_url: &str) -> Vec<String> {
    let document = Html::parse_document(html);
    let sel = match Selector::parse("a[href]") {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    let base = url::Url::parse(base_url).ok();

    document
        .select(&sel)
        .filter_map(|el| {
            let href = el.value().attr("href")?;
            if href.starts_with('#')
                || href.starts_with("javascript:")
                || href.starts_with("mailto:")
                || href.starts_with("data:")
                || href.starts_with("tel:")
                || href.starts_with("blob:")
            {
                return None;
            }
            if let Some(base) = &base {
                base.join(href).ok().map(|u| u.to_string())
            } else if href.starts_with("http") {
                Some(href.to_string())
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_article_content() {
        let html = r#"<html><body><nav>Nav</nav><article><p>Main content</p></article><footer>Foot</footer></body></html>"#;
        let content = extract_main_content(html);
        assert!(content.contains("Main content"));
    }

    #[test]
    fn extracts_title_and_description() {
        let html = r#"<html><head><title>Test Page</title><meta name="description" content="A test"></head><body></body></html>"#;
        let meta = extract_metadata(html);
        assert_eq!(meta.title.unwrap(), "Test Page");
        assert_eq!(meta.description.unwrap(), "A test");
    }

    #[test]
    fn extracts_og_metadata() {
        let html = r#"<html><head>
            <meta property="og:title" content="OG Title">
            <meta property="og:description" content="OG Desc">
            <meta property="og:image" content="https://img.com/pic.jpg">
            <link rel="canonical" href="https://example.com/canonical">
        </head><body></body></html>"#;
        let meta = extract_metadata(html);
        assert_eq!(meta.og_title.unwrap(), "OG Title");
        assert_eq!(meta.og_description.unwrap(), "OG Desc");
        assert_eq!(meta.og_image.unwrap(), "https://img.com/pic.jpg");
        assert_eq!(meta.canonical_url.unwrap(), "https://example.com/canonical");
    }

    #[test]
    fn extracts_links() {
        let html = r##"<html><body><a href="/page1">P1</a><a href="https://other.com">O</a><a href="#top">T</a></body></html>"##;
        let links = extract_links(html, "https://example.com");
        assert_eq!(links.len(), 2);
        assert!(links.contains(&"https://example.com/page1".to_string()));
        assert!(
            links.contains(&"https://other.com".to_string())
                || links.contains(&"https://other.com/".to_string())
        );
    }
}
