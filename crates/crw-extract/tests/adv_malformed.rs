//! Adversarial / malformed-HTML stress tests for
//! `crw_extract::readability::extract_images`.
//!
//! Every test feeds hostile input and asserts the function (a) does not panic
//! and (b) returns sensible output. We deliberately do NOT re-assert the
//! documented intentional behaviors (empty-src skip, verbatim absolutes,
//! data:/blob: kept, only `javascript:` filtered, WHATWG srcset comma rule,
//! dedup/alt-upgrade) as bugs — those are contract, exercised here only to
//! confirm they survive garbage around them.

use crw_core::types::ScrapedImage;
use crw_extract::readability::extract_images;

const BASE: &str = "https://example.com/dir/page.html";

fn urls(imgs: &[ScrapedImage]) -> Vec<String> {
    imgs.iter().map(|i| i.url.clone()).collect()
}

#[test]
fn unclosed_and_mismatched_tags() {
    let html = r#"<html><body><div><img src="a.png" alt="A"><section><img src="b.png"
        </div></span></article><img src="c.png" alt="C">"#;
    let imgs = extract_images(html, BASE);
    let u = urls(&imgs);
    // Parser recovery must still surface the well-formed srcs.
    assert!(u.iter().any(|s| s.ends_with("/dir/a.png")));
    assert!(u.iter().any(|s| s.ends_with("/dir/c.png")));
}

#[test]
fn deeply_nested_1500_deep() {
    let mut html = String::from("<html><body>");
    for _ in 0..1500 {
        html.push_str("<div>");
    }
    html.push_str(r#"<img src="deep.png" alt="deep">"#);
    for _ in 0..1500 {
        html.push_str("</div>");
    }
    html.push_str("</body></html>");
    let imgs = extract_images(&html, BASE);
    assert!(urls(&imgs).iter().any(|s| s.ends_with("/dir/deep.png")));
}

#[test]
fn enormous_attribute_value() {
    // A ~1MB junk alt plus a huge relative src path.
    let big_alt = "x".repeat(1_000_000);
    let big_path = "seg/".repeat(50_000);
    let html = format!(r#"<img src="{big_path}real.png" alt="{big_alt}">"#);
    let imgs = extract_images(&html, BASE);
    assert_eq!(imgs.len(), 1);
    assert!(imgs[0].url.ends_with("real.png"));
    assert_eq!(imgs[0].alt.as_deref().map(str::len), Some(1_000_000));
}

#[test]
fn thousands_of_srcset_candidates() {
    let mut ss = String::new();
    for i in 0..5000 {
        ss.push_str(&format!("img{i}.png {}w, ", i + 1));
    }
    ss.push_str("last.png 6000w");
    let html = format!(r#"<img srcset="{ss}" alt="many">"#);
    let imgs = extract_images(&html, BASE);
    // 5000 + last, all distinct -> all resolved.
    assert_eq!(imgs.len(), 5001);
    assert!(
        imgs.iter()
            .all(|i| i.url.starts_with("https://example.com/dir/"))
    );
}

#[test]
fn null_bytes_and_control_chars_in_attrs() {
    let html = "<img src=\"a\0b.png\" alt=\"al\u{0001}t\x07\"><img src=\"clean.png\" alt=\"ok\">";
    let imgs = extract_images(html, BASE);
    // Must not panic; the clean one must survive.
    assert!(urls(&imgs).iter().any(|s| s.ends_with("clean.png")));
}

#[test]
fn broken_html_entities() {
    let html = r#"<img src="a.png?x=1&notanentity&amp=2&#;&#xZZ;" alt="&foo &amp; &#65;">"#;
    let imgs = extract_images(html, BASE);
    assert_eq!(imgs.len(), 1);
    assert!(imgs[0].url.contains("a.png"));
}

#[test]
fn img_inside_template_svg_and_comment() {
    let html = r#"
        <template><img src="tpl.png" alt="tpl"></template>
        <svg><image href="svg.png"/><img src="svgimg.png"></svg>
        <!-- <img src="commented.png" alt="c"> -->
        <img src="real.png" alt="real">
    "#;
    let imgs = extract_images(html, BASE);
    let u = urls(&imgs);
    // The real, non-hidden img must be present; commented one must NOT.
    assert!(u.iter().any(|s| s.ends_with("real.png")));
    assert!(!u.iter().any(|s| s.contains("commented.png")));
}

#[test]
fn malformed_base_href_degrades_to_absolute_only() {
    // A garbage base must not panic and must not swallow absolutes.
    let html = r#"<base href="ht!tp://[::bad url">
        <img src="rel.png"><img src="https://cdn.example.org/abs.png">"#;
    let imgs = extract_images(html, BASE);
    let u = urls(&imgs);
    assert!(u.iter().any(|s| s == "https://cdn.example.org/abs.png"));
}

#[test]
fn multiple_base_tags_first_wins() {
    let html = r#"<base href="https://first.example/a/">
        <base href="https://second.example/b/">
        <img src="rel.png">"#;
    let imgs = extract_images(html, BASE);
    // scraper's select yields document order; first base[href] is used.
    assert_eq!(
        urls(&imgs),
        vec!["https://first.example/a/rel.png".to_string()]
    );
}

#[test]
fn base_href_empty_and_whitespace() {
    let html = r#"<base href="   "><img src="rel.png">"#;
    let imgs = extract_images(html, BASE);
    // Empty/ws base joins to nothing usable -> falls back to doc base.
    assert!(urls(&imgs).iter().any(|s| s.contains("rel.png")));
}

#[test]
fn picture_with_many_nested_sources() {
    let mut html = String::from("<picture>");
    for i in 0..1000 {
        html.push_str(&format!(r#"<source srcset="s{i}.png {}w">"#, i + 1));
    }
    html.push_str(r#"<img src="fallback.png" alt="pic"></picture>"#);
    let imgs = extract_images(&html, BASE);
    // 1000 sources + 1 fallback img.
    assert_eq!(imgs.len(), 1001);
}

#[test]
fn background_image_unbalanced_parens() {
    let html = r#"<div style="background-image: url('a.png'"></div>
        <div style="background: url(b.png"></div>
        <div style="background:url(c.png)"></div>"#;
    let imgs = extract_images(html, BASE);
    // Must not panic. The well-formed c.png resolves.
    assert!(urls(&imgs).iter().any(|s| s.ends_with("c.png")));
}

#[test]
fn background_image_many_url_calls() {
    let mut style = String::new();
    for i in 0..2000 {
        style.push_str(&format!("url(bg{i}.png), "));
    }
    let html = format!(r#"<div style="background-image: {style}url(last.png)"></div>"#);
    let imgs = extract_images(&html, BASE);
    assert_eq!(imgs.len(), 2001);
}

#[test]
fn extremely_long_data_uri() {
    let payload = "A".repeat(3_000_000);
    let html = format!(r#"<img src="data:image/png;base64,{payload}" alt="huge">"#);
    let imgs = extract_images(&html, BASE);
    assert_eq!(imgs.len(), 1);
    assert!(imgs[0].url.starts_with("data:image/png;base64,"));
    assert_eq!(
        imgs[0].url.len(),
        "data:image/png;base64,".len() + 3_000_000
    );
}

#[test]
fn data_uri_comma_in_srcset_not_split() {
    // WHATWG rule: comma inside data: URL does not split the candidate.
    let html = r#"<img srcset="data:image/gif;base64,R0lGODlhAQ== 1x, /real.png 2x" alt="lazy">"#;
    let imgs = extract_images(html, BASE);
    let u = urls(&imgs);
    assert!(
        u.iter()
            .any(|s| s.starts_with("data:image/gif;base64,R0lGODlhAQ=="))
    );
    assert!(u.iter().any(|s| s.ends_with("/real.png")));
    // No phantom fragment like "base64" or "R0lGODlhAQ==" split off.
    assert!(!u.iter().any(|s| s.ends_with("/dir/base64")));
}

#[test]
fn non_ascii_and_emoji_in_alt_and_url() {
    let html = "<img src=\"/görsel/naïve-☃.png\" alt=\"café 😀 résumé\">\
        <img src=\"https://例え.jp/画像.png\" alt=\"日本語\">";
    let imgs = extract_images(html, BASE);
    assert!(!imgs.is_empty());
    // alt preserved as-is (trimmed only).
    assert!(
        imgs.iter()
            .any(|i| i.alt.as_deref() == Some("café 😀 résumé"))
    );
}

#[test]
fn img_with_no_attributes() {
    let html = "<img><img alt><img src><img src=''><p>text</p>";
    let imgs = extract_images(html, BASE);
    // No usable src anywhere -> empty, no panic.
    assert!(imgs.is_empty());
}

#[test]
fn whitespace_only_src_skipped() {
    let html = "<img src=\"   \n\t \" alt=\"blank\"><img src=\"good.png\">";
    let imgs = extract_images(html, BASE);
    assert_eq!(
        urls(&imgs),
        vec!["https://example.com/dir/good.png".to_string()]
    );
}

#[test]
fn javascript_scheme_filtered_case_insensitive() {
    let html = r#"<img src="JavaScript:alert(1)"><img src="  javascript:void(0)">
        <img src="ok.png">"#;
    let imgs = extract_images(html, BASE);
    assert_eq!(
        urls(&imgs),
        vec!["https://example.com/dir/ok.png".to_string()]
    );
}

#[test]
fn massive_duplicate_urls_dedup_linear() {
    // 20k identical imgs, last one carries alt -> single entry, alt upgraded.
    let mut html = String::new();
    for _ in 0..20_000 {
        html.push_str(r#"<img src="dup.png">"#);
    }
    html.push_str(r#"<img src="dup.png" alt="finally">"#);
    let imgs = extract_images(&html, BASE);
    assert_eq!(imgs.len(), 1);
    assert_eq!(imgs[0].alt.as_deref(), Some("finally"));
}

#[test]
fn garbage_bytes_and_random_angle_brackets() {
    let html = "<<<>>><img src=\"a.png\"<<< alt=\"x\">>> < img src=b.png > <img/src=c.png>";
    let imgs = extract_images(html, BASE);
    // Must not panic; well-formed a.png should be found.
    assert!(urls(&imgs).iter().any(|s| s.ends_with("a.png")));
}

#[test]
fn empty_and_tiny_inputs() {
    for html in ["", " ", "\0", "<", "<html>", "not html at all", "<img"] {
        let imgs = extract_images(html, BASE);
        assert!(imgs.is_empty(), "expected empty for {html:?}");
    }
}

#[test]
fn invalid_base_url_argument() {
    // A malformed base_url must degrade to absolute-only, never panic.
    let html = r#"<img src="rel.png"><img src="https://cdn.example.org/abs.png">"#;
    let imgs = extract_images(html, "not a url");
    let u = urls(&imgs);
    assert!(u.iter().any(|s| s == "https://cdn.example.org/abs.png"));
    // rel.png cannot resolve without a valid base -> dropped, not panicked.
    assert!(!u.iter().any(|s| s.contains("rel.png")));
}

#[test]
fn srcset_only_commas_and_whitespace() {
    // First srcset is pure commas/ws -> no tokens, no panic.
    // Second has no whitespace, so per the WHATWG URL step the whole run
    // `a.png,,b.png` is ONE url token (commas only split after ws/descriptor);
    // trailing commas are stripped. This is the documented deliberate rule.
    let html = r#"<img srcset="   ,,,  , ,, ">
        <img srcset=",a.png,,b.png,, c.png 2x">"#;
    let imgs = extract_images(html, BASE);
    let u = urls(&imgs);
    assert!(u.iter().any(|s| s.ends_with("/dir/a.png,,b.png")));
    assert!(u.iter().any(|s| s.ends_with("/dir/c.png")));
    // No junk phantom split off the comma-joined run.
    assert!(!u.iter().any(|s| s.ends_with("/dir/b.png")));
}
