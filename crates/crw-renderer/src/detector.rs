/// Heuristic: does the HTML look like an SPA shell that needs JS rendering?
pub fn needs_js_rendering(html: &str) -> bool {
    // Only check first 50KB for performance on large pages
    let check_len = html.len().min(50_000);
    let lower = html[..check_len].to_lowercase();
    let body_len = extract_body_text_len(&lower);

    // Very short body text + presence of JS framework indicators.
    if body_len < 100 {
        let spa_indicators = [
            "id=\"root\"",
            "id=\"app\"",
            "id=\"__next\"",
            "id=\"__nuxt\"",
            "ng-app",
            "data-reactroot",
            "<script src",
            "window.__INITIAL_STATE__",
            "__NEXT_DATA__",
        ];
        if spa_indicators.iter().any(|ind| lower.contains(ind)) {
            return true;
        }
    }

    // Noscript tag with meaningful content suggests JS is needed.
    if lower.contains("<noscript>") && lower.contains("enable javascript") {
        return true;
    }

    false
}

/// Rough estimate of text content length inside <body>.
fn extract_body_text_len(html: &str) -> usize {
    let body_start = html.find("<body").and_then(|i| html[i..].find('>').map(|j| i + j + 1));
    let body_end = html.rfind("</body>");

    if let (Some(start), Some(end)) = (body_start, body_end) {
        if start < end {
            let body = &html[start..end];
            // Strip all tags and count remaining chars.
            let mut in_tag = false;
            let text_len = body
                .chars()
                .filter(|&c| {
                    if c == '<' {
                        in_tag = true;
                        false
                    } else if c == '>' {
                        in_tag = false;
                        false
                    } else {
                        !in_tag && !c.is_whitespace()
                    }
                })
                .count();
            return text_len;
        }
    }
    // If we can't find body tags, assume it has content.
    1000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_spa_shell() {
        let html = r#"<html><head></head><body><div id="root"></div><script src="/app.js"></script></body></html>"#;
        assert!(needs_js_rendering(html));
    }

    #[test]
    fn static_page_no_js_needed() {
        let html = r#"<html><body><article><h1>Hello World</h1><p>This is a long article with plenty of text content to read and enjoy. It has multiple paragraphs and lots of useful information.</p></article></body></html>"#;
        assert!(!needs_js_rendering(html));
    }
}
