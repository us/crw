use regex::Regex;
use std::sync::LazyLock;

/// Convert HTML to Markdown using htmd (turndown.js-inspired converter).
pub fn html_to_markdown(html: &str) -> String {
    let md = htmd::convert(html).unwrap_or_default();
    let md = strip_anchor_artifacts(&md);
    convert_indented_code_to_fenced(&md)
}

/// Convert 4-space indented code blocks to fenced (```) code blocks.
/// Fenced blocks are unambiguous and easier for RAG pipelines to parse.
fn convert_indented_code_to_fenced(md: &str) -> String {
    let mut result = String::with_capacity(md.len());
    let mut code_lines: Vec<&str> = Vec::new();
    let mut in_fenced = false;

    for line in md.lines() {
        // Track existing fenced code blocks to avoid double-fencing.
        if line.trim_start().starts_with("```") {
            in_fenced = !in_fenced;
            if !code_lines.is_empty() {
                flush_code_block(&mut result, &mut code_lines);
            }
            result.push_str(line);
            result.push('\n');
            continue;
        }

        if in_fenced {
            result.push_str(line);
            result.push('\n');
            continue;
        }

        let is_code_indent = line.starts_with("    ") || line.starts_with('\t');
        let is_blank = line.trim().is_empty();

        if is_code_indent {
            let stripped = if let Some(s) = line.strip_prefix("    ") {
                s
            } else if let Some(s) = line.strip_prefix('\t') {
                s
            } else {
                line
            };
            code_lines.push(stripped);
        } else if is_blank && !code_lines.is_empty() {
            code_lines.push("");
        } else {
            if !code_lines.is_empty() {
                flush_code_block(&mut result, &mut code_lines);
            }
            result.push_str(line);
            result.push('\n');
        }
    }

    if !code_lines.is_empty() {
        flush_code_block(&mut result, &mut code_lines);
    }

    // Match original trailing newline behavior.
    if result.ends_with('\n') && !md.ends_with('\n') {
        result.pop();
    }

    result
}

fn flush_code_block(result: &mut String, code_lines: &mut Vec<&str>) {
    while code_lines.last() == Some(&"") {
        code_lines.pop();
    }
    if !code_lines.is_empty() {
        result.push_str("```\n");
        for line in code_lines.iter() {
            result.push_str(line);
            result.push('\n');
        }
        result.push_str("```\n");
    }
    code_lines.clear();
}

/// Remove empty anchor links, pilcrow signs (¶), section signs (§), and other
/// anchor-link artifacts that HTML-to-Markdown converters carry over from
/// header anchor links.
fn strip_anchor_artifacts(md: &str) -> String {
    // Remove empty anchor links: [](#id), [](#id "title"), [¶](#id)
    static EMPTY_ANCHOR_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"\[¶?\]\(#[^)]*\)"#).unwrap());

    let cleaned = EMPTY_ANCHOR_RE.replace_all(md, "");
    cleaned
        .replace('\u{00b6}', "") // pilcrow ¶
        .replace(" \u{00a7}", "") // section sign § (preceded by space)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_basic_html() {
        let html = "<h1>Title</h1><p>Paragraph with <strong>bold</strong> text.</p>";
        let md = html_to_markdown(html);
        assert!(md.contains("# Title"));
        assert!(md.contains("**bold**"));
    }

    #[test]
    fn strips_pilcrow_from_headers() {
        let html = r##"<h2>Section <a href="#section">¶</a></h2>"##;
        let md = html_to_markdown(html);
        assert!(!md.contains('\u{00b6}'));
        assert!(md.contains("Section"));
    }

    #[test]
    fn strips_empty_anchor_links() {
        let input = "## Heading [](#heading) rest\n\nSome [¶](#foo \"title\") text";
        let result = strip_anchor_artifacts(input);
        assert!(!result.contains("[](#"));
        assert!(!result.contains("[¶](#"));
        assert!(result.contains("## Heading  rest"));
        assert!(result.contains("Some  text"));
    }

    #[test]
    fn converts_links() {
        let html = r#"<p><a href="https://example.com">Link</a></p>"#;
        let md = html_to_markdown(html);
        assert!(md.contains("[Link](https://example.com)"));
    }
}
