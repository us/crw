use scraper::Html;

/// Extract plain text from HTML, collapsing whitespace.
pub fn html_to_plaintext(html: &str) -> String {
    let document = Html::parse_document(html);
    let text: String = document.root_element().text().collect();
    // Collapse whitespace.
    let mut result = String::with_capacity(text.len());
    let mut prev_was_space = true;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if ch == '\n' {
                if !prev_was_space {
                    result.push('\n');
                    prev_was_space = true;
                }
            } else if !prev_was_space {
                result.push(' ');
                prev_was_space = true;
            }
        } else {
            result.push(ch);
            prev_was_space = false;
        }
    }
    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_plain_text() {
        let html = "<html><body><h1>Title</h1><p>Hello   world</p></body></html>";
        let text = html_to_plaintext(html);
        assert!(text.contains("Title"));
        assert!(text.contains("Hello world"));
    }
}
