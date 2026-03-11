use lol_html::{RewriteStrSettings, element, rewrite_str};
use scraper::{Html, Selector};
use std::collections::HashSet;

/// Clean HTML by stripping scripts, styles, and optionally non-content elements.
/// Then apply include_tags/exclude_tags via scraper.
pub fn clean_html(
    html: &str,
    only_main_content: bool,
    include_tags: &[String],
    exclude_tags: &[String],
) -> Result<String, String> {
    // Phase 1: lol_html streaming removal of always-unwanted tags.
    let mut handlers = vec![
        element!("script", |el| {
            el.remove();
            Ok(())
        }),
        element!("style", |el| {
            el.remove();
            Ok(())
        }),
        element!("noscript", |el| {
            el.remove();
            Ok(())
        }),
        element!("iframe", |el| {
            el.remove();
            Ok(())
        }),
        element!("svg", |el| {
            el.remove();
            Ok(())
        }),
        element!("canvas", |el| {
            el.remove();
            Ok(())
        }),
    ];

    if only_main_content {
        handlers.push(element!("nav", |el| {
            el.remove();
            Ok(())
        }));
        handlers.push(element!("footer", |el| {
            el.remove();
            Ok(())
        }));
        handlers.push(element!("header", |el| {
            el.remove();
            Ok(())
        }));
        handlers.push(element!("aside", |el| {
            el.remove();
            Ok(())
        }));
        handlers.push(element!("menu", |el| {
            el.remove();
            Ok(())
        }));

        // Remove elements whose class or id matches common non-content patterns.
        // Covers sidebars, TOC, navigation, ads, related/recommended sections,
        // cookie banners, share widgets, and comment sections.
        handlers.push(element!("*", |el| {
            let class = el.get_attribute("class").unwrap_or_default().to_lowercase();
            let id = el.get_attribute("id").unwrap_or_default().to_lowercase();
            let combined = format!("{class} {id}");

            const NOISE_PATTERNS: &[&str] = &[
                "sidebar",
                "toc",
                "table-of-contents",
                "tableofcontents",
                "infobox",
                "navbox",
                "nav-box",
                "navigation",
                "breadcrumb",
                "cookie",
                "consent",
                "banner",
                "share",
                "social",
                "related",
                "recommended",
                "comment",
                "disqus",
                "ad-",
                "ads-",
                "advert",
                "popup",
                "modal",
                "newsletter",
                "subscribe",
                "printfooter",
                "catlinks",
                "mw-panel",
                "mw-navigation",
                "sitesub",
                "jump-to-nav",
            ];

            if NOISE_PATTERNS.iter().any(|p| combined.contains(p)) {
                el.remove();
            }

            Ok(())
        }));
    }

    let mut result = rewrite_str(
        html,
        RewriteStrSettings {
            element_content_handlers: handlers,
            ..Default::default()
        },
    )
    .map_err(|e| e.to_string())?;

    // Phase 2: If include_tags specified, only keep content matching those selectors.
    if !include_tags.is_empty() {
        result = keep_only_selectors(&result, include_tags);
    }

    // Phase 3: Apply exclude_tags — parse again and collect text/html without excluded.
    if !exclude_tags.is_empty() {
        result = remove_by_selectors(&result, exclude_tags);
    }

    Ok(result)
}

/// Keep only the HTML of elements matching any of the given CSS selectors.
fn keep_only_selectors(html: &str, selectors: &[String]) -> String {
    let doc = Html::parse_document(html);
    let mut parts = Vec::new();

    for sel_str in selectors {
        match Selector::parse(sel_str) {
            Ok(sel) => {
                for el in doc.select(&sel) {
                    parts.push(el.html());
                }
            }
            Err(e) => {
                tracing::warn!("Invalid CSS selector '{}': {:?}", sel_str, e);
            }
        }
    }

    if parts.is_empty() {
        return html.to_string();
    }

    parts.join("\n")
}

/// Remove elements matching CSS selectors from the document.
/// Re-serializes the tree, skipping matched subtrees via tree node indices.
fn remove_by_selectors(html: &str, selectors: &[String]) -> String {
    let doc = Html::parse_document(html);

    // Collect pointers to matched elements for exclusion.
    // SAFETY: All pointers point into `doc` which lives for the entire function scope.
    // We only compare pointers (never dereference), so this is safe as long as `doc` is alive.
    let mut skip_ptrs: HashSet<*const scraper::node::Element> = HashSet::new();
    for sel_str in selectors {
        match Selector::parse(sel_str) {
            Ok(sel) => {
                for el in doc.select(&sel) {
                    skip_ptrs.insert(el.value() as *const _);
                }
            }
            Err(e) => {
                tracing::warn!("Invalid CSS selector '{}': {:?}", sel_str, e);
            }
        }
    }

    if skip_ptrs.is_empty() {
        return html.to_string();
    }

    // Re-serialize the root element, skipping excluded subtrees.
    // Pre-allocate output based on input size.
    let root = doc.root_element();
    let mut out = String::with_capacity(html.len());
    collect_excluding(&root, &skip_ptrs, &mut out);
    out
}

fn is_excluded(
    el: &scraper::ElementRef,
    skip_ptrs: &HashSet<*const scraper::node::Element>,
) -> bool {
    let ptr = el.value() as *const scraper::node::Element;
    skip_ptrs.contains(&ptr)
}

fn collect_excluding(
    element: &scraper::ElementRef,
    skip_ptrs: &HashSet<*const scraper::node::Element>,
    out: &mut String,
) {
    if is_excluded(element, skip_ptrs) {
        return;
    }

    let el = element.value();
    out.push('<');
    out.push_str(&el.name.local);
    for (name, value) in el.attrs() {
        out.push(' ');
        out.push_str(name);
        out.push_str("=\"");
        out.push_str(&value.replace('"', "&quot;"));
        out.push('"');
    }
    out.push('>');

    for child in element.children() {
        match child.value() {
            scraper::node::Node::Text(text) => {
                out.push_str(text);
            }
            scraper::node::Node::Element(_) => {
                if let Some(child_el) = scraper::ElementRef::wrap(child) {
                    collect_excluding(&child_el, skip_ptrs, out);
                }
            }
            _ => {}
        }
    }

    let self_closing = matches!(
        &*el.name.local,
        "br" | "hr"
            | "img"
            | "input"
            | "meta"
            | "link"
            | "area"
            | "base"
            | "col"
            | "embed"
            | "source"
            | "track"
            | "wbr"
    );
    if !self_closing {
        out.push_str("</");
        out.push_str(&el.name.local);
        out.push('>');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_scripts_and_styles() {
        let html =
            r#"<html><body><script>alert(1)</script><p>Hello</p><style>x{}</style></body></html>"#;
        let result = clean_html(html, false, &[], &[]).unwrap();
        assert!(!result.contains("<script>"));
        assert!(!result.contains("<style>"));
        assert!(result.contains("Hello"));
    }

    #[test]
    fn strips_nav_footer_in_main_content_mode() {
        let html = r#"<body><nav>Menu</nav><article>Content</article><footer>Foot</footer></body>"#;
        let result = clean_html(html, true, &[], &[]).unwrap();
        assert!(!result.contains("Menu"));
        assert!(!result.contains("Foot"));
        assert!(result.contains("Content"));
    }

    #[test]
    fn exclude_tags_removes_matching_elements() {
        let html = r#"<body><div class="ad">Ad stuff</div><p>Real content</p></body>"#;
        let result = clean_html(html, false, &[], &["div.ad".into()]).unwrap();
        assert!(!result.contains("Ad stuff"));
        assert!(result.contains("Real content"));
    }

    #[test]
    fn include_tags_keeps_only_matching() {
        let html =
            r#"<body><nav>Nav</nav><article><p>Article</p></article><footer>Foot</footer></body>"#;
        let result = clean_html(html, false, &["article".into()], &[]).unwrap();
        assert!(result.contains("Article"));
        assert!(!result.contains("Nav"));
        assert!(!result.contains("Foot"));
    }
}
