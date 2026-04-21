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

/// Check if rendered HTML is dominated by loading placeholders, spinners,
/// or chat-widget-only content. Used *after* JS rendering to detect cases
/// where the renderer returned early before the real content appeared
/// (common with slow React/Vite SPAs on underpowered renderers).
///
/// Markers are matched against *visible* body text only (tags and attributes
/// stripped) to avoid false positives from e.g. `<img alt="Loading...">` on
/// a page that actually has real content.
pub fn looks_like_loading_placeholder(html: &str) -> bool {
    // Bail out fast for large pages — real content dwarfs loading markers.
    if html.len() > 80_000 {
        return false;
    }
    let lower = html.to_lowercase();
    let body_stripped = body_html_without_scripts_lower(&lower);
    let body_text = visible_text_from_stripped_html(&body_stripped);
    let body_text_len = body_text.chars().filter(|c| !c.is_whitespace()).count();

    if body_text_len == 0 {
        return true;
    }

    // Short body + explicit loading text in VISIBLE text.
    if body_text_len < 400 {
        let loading_markers = [
            "loading...",
            "loading…",
            "please wait",
            "just a moment",
            "initializing",
            "preparing",
            "one moment",
        ];
        if loading_markers.iter().any(|m| body_text.contains(m)) {
            return true;
        }
    }

    // Very short body + spinner/loader DOM markers. Matched against body HTML
    // with <script>/<style> stripped, so inline JS like `'class="spinner"'`
    // does not trigger a false positive.
    if body_text_len < 200 {
        let spinner_markers = [
            "class=\"spinner",
            "class=\"loader",
            "class=\"loading",
            "class=\"preloader",
            "id=\"loader",
            "id=\"preloader",
            "aria-label=\"loading\"",
        ];
        if spinner_markers.iter().any(|m| body_stripped.contains(m)) {
            return true;
        }
    }

    false
}

/// Return the `<body>` of a lowercased HTML document with `<script>` and
/// `<style>` blocks removed. Remaining tags (and their attributes) are
/// preserved. Returns an empty string if no `<body>` is found.
fn body_html_without_scripts_lower(lower: &str) -> String {
    let body_start = lower
        .find("<body")
        .and_then(|i| lower[i..].find('>').map(|j| i + j + 1));
    let body_end = lower.rfind("</body>");

    let body = match (body_start, body_end) {
        (Some(start), Some(end)) if start < end => &lower[start..end],
        _ => return String::new(),
    };

    let stripped = strip_tag_blocks(body, "script");
    strip_tag_blocks(&stripped, "style")
}

/// Strip all HTML tags (open/close, with attributes) from an already
/// script/style-stripped HTML fragment. Whitespace is collapsed.
fn visible_text_from_stripped_html(stripped: &str) -> String {
    let mut text = String::with_capacity(stripped.len());
    let mut in_tag = false;
    let mut prev_ws = true;
    for ch in stripped.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' {
            in_tag = false;
        } else if !in_tag {
            if ch.is_whitespace() {
                if !prev_ws {
                    text.push(' ');
                    prev_ws = true;
                }
            } else {
                text.push(ch);
                prev_ws = false;
            }
        }
    }
    text
}

/// Rough estimate of non-whitespace text length inside `<body>` of a
/// lowercased HTML document. Returns `1000` as a "probably has content"
/// fallback if no `<body>` is found.
fn extract_body_text_len(lower: &str) -> usize {
    if !lower.contains("<body") {
        return 1000;
    }
    let stripped = body_html_without_scripts_lower(lower);
    visible_text_from_stripped_html(&stripped)
        .chars()
        .filter(|c| !c.is_whitespace())
        .count()
}

/// Remove all `<tag ...>...</tag>` blocks from HTML. The input is assumed
/// to be already lowercased (callers pass lowercased HTML).
fn strip_tag_blocks(html: &str, tag: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);
    let mut remaining = html;

    while let Some(start) = remaining.find(&open) {
        result.push_str(&remaining[..start]);
        let after_open = &remaining[start..];
        if let Some(end) = after_open.find(&close) {
            remaining = &after_open[end + close.len()..];
        } else {
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

    #[test]
    fn detects_loading_placeholder_text() {
        let html =
            r#"<html><body><div><p>Loading...</p><p>Hi! Ask me anything.</p></div></body></html>"#;
        assert!(looks_like_loading_placeholder(html));
    }

    #[test]
    fn detects_spinner_only_body() {
        let html = r#"<html><body><div class="spinner"></div></body></html>"#;
        assert!(looks_like_loading_placeholder(html));
    }

    #[test]
    fn real_content_not_placeholder() {
        let html = r#"<html><body><article><h1>Welcome to my creative space</h1><p>Waqar Bin Abrar is a full stack developer specializing in MERN stack and Flutter apps, building scalable digital solutions for clients worldwide.</p><p>With years of experience delivering production applications, he combines technical expertise with design sensibility.</p></article></body></html>"#;
        assert!(!looks_like_loading_placeholder(html));
    }

    #[test]
    fn logo_alt_loading_on_real_page_not_placeholder() {
        // Regression: "Loading..." inside an img alt attribute must NOT trigger
        // placeholder detection when the page has real visible content.
        let html = r#"<html><body>
            <header><img alt="Loading..." src="/logo.png"/></header>
            <article>
                <h1>Software Engineering Blog</h1>
                <p>Thoughts on distributed systems, programming languages, and the craft of writing software that lasts. New posts weekly.</p>
                <p>This site covers topics from Rust ownership to Kubernetes operators.</p>
            </article>
        </body></html>"#;
        assert!(!looks_like_loading_placeholder(html));
    }

    #[test]
    fn empty_body_is_placeholder() {
        let html = r#"<html><body></body></html>"#;
        assert!(looks_like_loading_placeholder(html));
    }

    #[test]
    fn large_page_never_placeholder() {
        let filler = "x".repeat(100_000);
        let html = format!("<html><body><p>Loading...</p>{filler}</body></html>");
        assert!(!looks_like_loading_placeholder(&html));
    }

    #[test]
    fn spinner_class_in_script_body_ignored() {
        // class="spinner" inside a <script> block must not trigger spinner detection,
        // since scripts are stripped before text-length measurement.
        let html = r#"<html><body><article><h1>Real Article</h1><p>This is a real article with substantial content about the topic at hand, providing useful information.</p><script>const x = 'class="spinner"';</script></article></body></html>"#;
        assert!(!looks_like_loading_placeholder(html));
    }
}
