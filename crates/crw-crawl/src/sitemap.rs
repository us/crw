use crw_core::error::{CrwError, CrwResult};
use scraper::{Html, Selector};

/// Fetch and parse a sitemap, returning all URLs found.
pub async fn fetch_sitemap(url: &str, client: &reqwest::Client) -> CrwResult<Vec<String>> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| CrwError::HttpError(e.to_string()))?;

    if !resp.status().is_success() {
        return Ok(vec![]);
    }

    let text = resp
        .text()
        .await
        .map_err(|e| CrwError::HttpError(e.to_string()))?;

    Ok(parse_sitemap(&text))
}

/// Parse sitemap XML and extract URLs. Handles both sitemap index and urlset.
pub fn parse_sitemap(xml: &str) -> Vec<String> {
    let document = Html::parse_document(xml);
    let mut urls = Vec::new();

    // Try <url><loc> (standard sitemap).
    if let Ok(sel) = Selector::parse("url > loc") {
        for el in document.select(&sel) {
            let text: String = el.text().collect();
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                urls.push(trimmed.to_string());
            }
        }
    }

    // Try <sitemap><loc> (sitemap index).
    if let Ok(sel) = Selector::parse("sitemap > loc") {
        for el in document.select(&sel) {
            let text: String = el.text().collect();
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                urls.push(trimmed.to_string());
            }
        }
    }

    // Fallback: look for <loc> anywhere.
    if urls.is_empty() {
        if let Ok(sel) = Selector::parse("loc") {
            for el in document.select(&sel) {
                let text: String = el.text().collect();
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    urls.push(trimmed.to_string());
                }
            }
        }
    }

    urls
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_urlset() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url><loc>https://example.com/page1</loc></url>
  <url><loc>https://example.com/page2</loc></url>
</urlset>"#;
        let urls = parse_sitemap(xml);
        assert_eq!(urls.len(), 2);
    }
}
