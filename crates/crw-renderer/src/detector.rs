/// Heuristic: does the HTML look like an SPA shell that needs JS rendering?
pub fn needs_js_rendering(html: &str) -> bool {
    // Check up to 500KB — some pages have huge <head> sections (CSS, preloaded data)
    // and the <body> may start well beyond 50KB.
    let check_len = html.len().min(500_000);
    let lower = html[..check_len].to_lowercase();
    let body_len = extract_body_text_len(&lower);

    // Very short body text + presence of JS framework indicators.
    // After stripping script/style, most SPA shells have very little actual text.
    if body_len < 200 {
        let spa_indicators = [
            "id=\"root\"",
            "id=\"app\"",
            "id=\"__next\"",
            "id=\"__nuxt\"",
            "id=\"__gatsby\"",
            "id=\"svelte\"",
            "ng-app",
            "data-reactroot",
            "<script src",
            "window.__initial_state__",
            "__next_data__",
            "window.__remixcontext",
            "window.__astro",
        ];
        if spa_indicators.iter().any(|ind| lower.contains(ind)) {
            return true;
        }
    }

    // Noscript tag with meaningful content suggests JS is needed.
    if lower.contains("<noscript>") && lower.contains("enable javascript") {
        return true;
    }

    // Framer / Webflow / other site builder markers (often fully JS-rendered)
    if body_len < 500 {
        let builder_indicators = [
            "framerusercontent.com",
            "webflow.io",
            "wixsite.com",
            "squarespace.com/universal",
        ];
        if builder_indicators.iter().any(|ind| lower.contains(ind)) {
            return true;
        }
    }

    false
}

/// Rough estimate of text content length inside <body>,
/// after stripping `<script>` and `<style>` blocks so that inline JS/CSS
/// doesn't inflate the count and mask SPA shells.
fn extract_body_text_len(html: &str) -> usize {
    let body_start = html
        .find("<body")
        .and_then(|i| html[i..].find('>').map(|j| i + j + 1));
    let body_end = html.rfind("</body>");

    if let (Some(start), Some(end)) = (body_start, body_end)
        && start < end
    {
        let body = &html[start..end];
        // First strip <script>...</script> and <style>...</style> content
        let stripped = strip_tag_blocks(body, "script");
        let stripped = strip_tag_blocks(&stripped, "style");
        // Then count non-tag, non-whitespace chars
        let mut in_tag = false;
        let text_len = stripped
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
    // If we can't find body tags, assume it has content.
    1000
}

/// Remove all `<tag ...>...</tag>` blocks (case-insensitive) from HTML.
fn strip_tag_blocks(html: &str, tag: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);
    let mut remaining = html;

    while let Some(start) = remaining
        .find(&open)
        .or_else(|| remaining.find(&open.to_uppercase()))
    {
        result.push_str(&remaining[..start]);
        let after_open = &remaining[start..];
        if let Some(end) = after_open
            .find(&close)
            .or_else(|| after_open.find(&close.to_uppercase()))
        {
            remaining = &after_open[end + close.len()..];
        } else {
            // No closing tag found — skip to end
            remaining = "";
            break;
        }
    }
    result.push_str(remaining);
    result
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
