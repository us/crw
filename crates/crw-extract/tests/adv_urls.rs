//! Adversarial URL-resolution stress tests for
//! `crw_extract::readability::extract_images`.
//!
//! The engine intentionally mirrors Firecrawl's native `resolve_image_url` /
//! `_extract_images` (`competitors/firecrawl/apps/api/native/src/html.rs`) so
//! the Firecrawl-compat `/v2` surface stays a byte-for-byte drop-in. Each case
//! asserts the EXACT resolved URL, and — for every input that is not one of the
//! two documented deliberate divergences — cross-checks that crw's output equals
//! what a faithful re-implementation of Firecrawl's resolver (`fc_resolve`
//! below) produces from the same input, validating drop-in parity.
//!
//! Documented DELIBERATE divergences (asserted, not treated as bugs):
//!   1. empty / whitespace-only `src`: crw trims and skips it; Firecrawl joins
//!      the raw value (`base_href.join("")` = the PAGE URL) and emits that junk.
//!   2. `srcset` tokenization: crw uses the WHATWG URL step so a comma inside a
//!      `data:` URI stays whole; Firecrawl's naive `split(',')` shreds it.

use crw_extract::readability::extract_images;
use url::Url;

/// Resolve one `src` exactly the way Firecrawl's native `_extract_images` does
/// (branch-for-branch + its final filter), using the same `url` crate crw uses.
/// Returns `None` when Firecrawl would not emit the URL. `base_href` is the raw
/// `<base href>` attribute if present.
fn fc_resolve(src: &str, base_url: &str, base_href: Option<&str>) -> Option<String> {
    let base = Url::parse(base_url).ok()?;
    let base_href_url = match base_href {
        Some(h) => base.join(h).unwrap_or_else(|_| base.clone()),
        None => base.clone(),
    };
    let resolved = if src.starts_with("data:")
        || src.starts_with("blob:")
        || src.starts_with("http://")
        || src.starts_with("https://")
    {
        // data:/blob: and absolute http(s) are all kept verbatim by Firecrawl.
        src.to_string()
    } else if src.starts_with("//") {
        base.join(src).ok()?.to_string()
    } else {
        base_href_url.join(src).ok()?.to_string()
    };
    // Firecrawl's final filter pass.
    if resolved.to_lowercase().starts_with("javascript:") {
        return None;
    }
    if resolved.is_empty() {
        return None;
    }
    if resolved.starts_with("data:")
        || resolved.starts_with("blob:")
        || Url::parse(&resolved).is_ok()
    {
        Some(resolved)
    } else {
        None
    }
}

/// Single-`<img src>` page.
fn img(src: &str) -> String {
    format!(r#"<html><body><img src="{src}"></body></html>"#)
}

/// URLs of a one-image page, asserting exactly one image was produced.
fn one(html: &str, base: &str) -> String {
    let imgs = extract_images(html, base);
    assert_eq!(imgs.len(), 1, "expected exactly one image, got {imgs:?}");
    imgs[0].url.clone()
}

fn urls(html: &str, base: &str) -> Vec<String> {
    extract_images(html, base)
        .into_iter()
        .map(|i| i.url)
        .collect()
}

/// crw's resolution of a bare `<img src>` equals Firecrawl's, and equals the
/// literal we expect. Use only for parity-eligible (non-divergent) inputs.
fn assert_parity(src: &str, base: &str, base_href: Option<&str>, expected: &str) {
    let got = one(&img(src), base);
    assert_eq!(got, expected, "crw output for src={src:?}");
    assert_eq!(
        Some(got),
        fc_resolve(src, base, base_href),
        "Firecrawl parity for src={src:?}"
    );
}

// ---------------------------------------------------------------------------
// <base href>: relative vs absolute
// ---------------------------------------------------------------------------

#[test]
fn relative_src_no_base_href() {
    assert_parity(
        "foo/bar.png",
        "https://example.com/dir/page.html",
        None,
        "https://example.com/dir/foo/bar.png",
    );
}

#[test]
fn relative_base_href_relative_src() {
    let html = r#"<html><head><base href="/cdn/"></head><body><img src="a.png"></body></html>"#;
    assert_eq!(
        one(html, "https://example.com/dir/page"),
        "https://example.com/cdn/a.png"
    );
    // Parity: Firecrawl resolves `<base href>` against the page URL the same way.
    assert_eq!(
        fc_resolve("a.png", "https://example.com/dir/page", Some("/cdn/")).as_deref(),
        Some("https://example.com/cdn/a.png")
    );
}

#[test]
fn absolute_base_href_relative_src() {
    let html = r#"<html><head><base href="https://cdn.example.net/assets/"></head><body><img src="a.png"></body></html>"#;
    assert_eq!(
        one(html, "https://example.com/page"),
        "https://cdn.example.net/assets/a.png"
    );
    assert_eq!(
        fc_resolve(
            "a.png",
            "https://example.com/page",
            Some("https://cdn.example.net/assets/")
        )
        .as_deref(),
        Some("https://cdn.example.net/assets/a.png")
    );
}

#[test]
fn base_href_relative_resolved_against_relative_page_degrades() {
    // A relative page base_url (no scheme) cannot be parsed -> no doc base ->
    // relative src cannot resolve. Graceful: empty, no panic.
    let html = r#"<html><head><base href="/cdn/"></head><body><img src="a.png"></body></html>"#;
    assert!(urls(html, "/some/relative/path").is_empty());
}

// ---------------------------------------------------------------------------
// <base href> scheme vs protocol-relative img: PAGE scheme must win
// ---------------------------------------------------------------------------

#[test]
fn protocol_relative_inherits_page_scheme_not_base_href_scheme() {
    // <base href> is http://, page is https://. A protocol-relative img must
    // inherit the PAGE scheme (https), because `//host` joins the doc URL, not
    // the <base href>.
    let html = r#"<html><head><base href="http://cdn.example.net/"></head><body><img src="//img.host/x.png"></body></html>"#;
    assert_eq!(
        one(html, "https://example.com/page"),
        "https://img.host/x.png"
    );
    // Firecrawl joins `//host` against base_url (page) too -> same result.
    assert_eq!(
        fc_resolve(
            "//img.host/x.png",
            "https://example.com/page",
            Some("http://cdn.example.net/")
        )
        .as_deref(),
        Some("https://img.host/x.png")
    );
}

#[test]
fn protocol_relative_plain_page_scheme() {
    assert_parity(
        "//cdn.host/x.png",
        "http://example.com/",
        None,
        "http://cdn.host/x.png",
    );
}

// ---------------------------------------------------------------------------
// Absolute http(s):// kept VERBATIM (no canonicalization)
// ---------------------------------------------------------------------------

#[test]
fn absolute_https_kept_verbatim_no_normalization() {
    // Mixed-case host, unresolved dot segment, query — all preserved verbatim.
    assert_parity(
        "https://EXAMPLE.com/A/../B?x=1",
        "https://example.com/",
        None,
        "https://EXAMPLE.com/A/../B?x=1",
    );
}

#[test]
fn absolute_with_userinfo_port_query_fragment_verbatim() {
    assert_parity(
        "https://user:pass@example.com:8443/p?q=1&x=2#frag",
        "https://example.com/",
        None,
        "https://user:pass@example.com:8443/p?q=1&x=2#frag",
    );
}

#[test]
fn absolute_percent_encoded_path_verbatim() {
    assert_parity(
        "https://example.com/a%20b/c%2Fd.png",
        "https://example.com/",
        None,
        "https://example.com/a%20b/c%2Fd.png",
    );
}

// ---------------------------------------------------------------------------
// Uppercase scheme falls through to join (deliberate, case-sensitive prefix)
// ---------------------------------------------------------------------------

#[test]
fn uppercase_scheme_goes_through_join_and_is_canonicalized() {
    // `HTTPS://` fails the case-sensitive `starts_with("https://")` check, so it
    // is treated as relative and handed to `join`, which parses it as an
    // absolute URL and lowercases the scheme. Result differs from verbatim.
    assert_parity(
        "HTTPS://Example.com/x.png",
        "https://example.com/dir/",
        None,
        "https://example.com/x.png",
    );
}

// ---------------------------------------------------------------------------
// IDN / unicode hosts
// ---------------------------------------------------------------------------

#[test]
fn idn_host_absolute_kept_verbatim() {
    // Absolute -> verbatim; the unicode host survives Url::parse validation and
    // is NOT punycoded.
    assert_parity(
        "https://ドメイン.jp/img.png",
        "https://example.com/",
        None,
        "https://ドメイン.jp/img.png",
    );
}

#[test]
fn idn_host_via_relative_base_is_punycoded() {
    // Resolved through join (unicode <base href>), so the host IS punycoded.
    let html = r#"<html><head><base href="https://café.fr/x/"></head><body><img src="a.png"></body></html>"#;
    assert_eq!(
        one(html, "https://example.com/"),
        "https://xn--caf-dma.fr/x/a.png"
    );
    assert_eq!(
        fc_resolve("a.png", "https://example.com/", Some("https://café.fr/x/")).as_deref(),
        Some("https://xn--caf-dma.fr/x/a.png")
    );
}

// ---------------------------------------------------------------------------
// Relative resolution: paths, spaces, query/fragment, traversal
// ---------------------------------------------------------------------------

#[test]
fn relative_dotdot_traversal() {
    assert_parity(
        "../../up.png",
        "https://example.com/a/b/c/page.html",
        None,
        "https://example.com/a/up.png",
    );
}

#[test]
fn relative_src_outer_spaces_trimmed() {
    // crw trims outer whitespace; url-crate join also strips it, so parity holds.
    assert_parity(
        "  photo.png  ",
        "https://example.com/dir/",
        None,
        "https://example.com/dir/photo.png",
    );
}

#[test]
fn relative_src_inner_space_percent_encoded() {
    assert_parity(
        "my photo.png",
        "https://example.com/dir/",
        None,
        "https://example.com/dir/my%20photo.png",
    );
}

#[test]
fn relative_src_with_query_and_fragment() {
    assert_parity(
        "pic.png?v=2&s=3#top",
        "https://example.com/dir/",
        None,
        "https://example.com/dir/pic.png?v=2&s=3#top",
    );
}

#[test]
fn relative_absolute_path_root() {
    assert_parity(
        "/root/x.png",
        "https://example.com/a/b/page",
        None,
        "https://example.com/root/x.png",
    );
}

// ---------------------------------------------------------------------------
// Scheme filtering: only javascript: dropped, case-insensitively
// ---------------------------------------------------------------------------

#[test]
fn javascript_scheme_lowercase_dropped() {
    assert!(urls(&img("javascript:alert(1)"), "https://example.com/").is_empty());
    assert_eq!(
        fc_resolve("javascript:alert(1)", "https://example.com/", None),
        None
    );
}

#[test]
fn javascript_scheme_uppercase_dropped() {
    assert!(urls(&img("JAVASCRIPT:alert(1)"), "https://example.com/").is_empty());
    assert_eq!(
        fc_resolve("JAVASCRIPT:alert(1)", "https://example.com/", None),
        None
    );
}

#[test]
fn vbscript_scheme_kept() {
    // Only javascript: is filtered; vbscript: survives (weird but documented).
    assert_parity(
        "vbscript:msgbox(1)",
        "https://example.com/",
        None,
        "vbscript:msgbox(1)",
    );
}

#[test]
fn file_scheme_kept() {
    assert_parity(
        "file:///etc/passwd",
        "https://example.com/",
        None,
        "file:///etc/passwd",
    );
}

#[test]
fn mailto_in_img_src_kept() {
    assert_parity(
        "mailto:foo@bar.com",
        "https://example.com/",
        None,
        "mailto:foo@bar.com",
    );
}

#[test]
fn tel_in_img_src_kept() {
    assert_parity(
        "tel:+1-800-555",
        "https://example.com/",
        None,
        "tel:+1-800-555",
    );
}

// ---------------------------------------------------------------------------
// Malformed / empty base_url: graceful degradation, no panic
// ---------------------------------------------------------------------------

#[test]
fn malformed_base_url_relative_src_degrades_empty() {
    assert!(urls(&img("a.png"), "not a valid url").is_empty());
}

#[test]
fn malformed_base_url_absolute_src_still_resolves() {
    // Absolute src needs no base, so it survives a garbage base_url.
    assert_eq!(
        one(&img("https://example.com/x.png"), "garbage-no-scheme"),
        "https://example.com/x.png"
    );
}

#[test]
fn empty_base_url_absolute_src() {
    assert_eq!(
        one(&img("https://example.com/x.png"), ""),
        "https://example.com/x.png"
    );
}

#[test]
fn empty_base_url_relative_src_dropped() {
    assert!(urls(&img("a.png"), "").is_empty());
}

// ---------------------------------------------------------------------------
// DELIBERATE divergence 1: empty / whitespace src
// ---------------------------------------------------------------------------

#[test]
fn empty_src_skipped_diverges_from_firecrawl() {
    // crw skips empty src entirely.
    assert!(urls(&img(""), "https://example.com/dir/page").is_empty());
    // Firecrawl would emit the PAGE URL (join("") = base_href) as a junk image.
    assert_eq!(
        fc_resolve("", "https://example.com/dir/page", None).as_deref(),
        Some("https://example.com/dir/page")
    );
}

#[test]
fn whitespace_only_src_skipped_diverges_from_firecrawl() {
    assert!(urls(&img("   "), "https://example.com/dir/page").is_empty());
    // Firecrawl trims via url-crate join -> also the page URL. Documented div.
    assert_eq!(
        fc_resolve("   ", "https://example.com/dir/page", None).as_deref(),
        Some("https://example.com/dir/page")
    );
}

// ---------------------------------------------------------------------------
// data: / blob: kept verbatim, special chars intact
// ---------------------------------------------------------------------------

#[test]
fn data_uri_with_special_chars_verbatim() {
    let src = "data:image/svg+xml;utf8,<svg xmlns='http://x'><circle r='5'/></svg>";
    // Note: no outer whitespace (trim is a no-op here); inner spaces/parens kept.
    assert_parity(src, "https://example.com/", None, src);
}

#[test]
fn blob_uri_verbatim() {
    let src = "blob:https://example.com/550e8400-e29b-41d4";
    assert_parity(src, "https://example.com/", None, src);
}

// ---------------------------------------------------------------------------
// srcset: data: URIs and mixed descriptors
// ---------------------------------------------------------------------------

#[test]
fn srcset_ordinary_descriptors_match_firecrawl() {
    let html = r#"<html><body><img srcset="a.jpg 480w, b.jpg 1080w, c.jpg 2x"></body></html>"#;
    let got = urls(html, "https://example.com/dir/");
    assert_eq!(
        got,
        vec![
            "https://example.com/dir/a.jpg".to_string(),
            "https://example.com/dir/b.jpg".to_string(),
            "https://example.com/dir/c.jpg".to_string(),
        ]
    );
    // Parity: Firecrawl's naive split yields the same tokens for ordinary srcsets.
    for (tok, exp) in [("a.jpg", &got[0]), ("b.jpg", &got[1]), ("c.jpg", &got[2])] {
        assert_eq!(
            fc_resolve(tok, "https://example.com/dir/", None).as_ref(),
            Some(exp)
        );
    }
}

#[test]
fn srcset_data_uri_kept_whole_diverges_from_firecrawl() {
    // DELIBERATE divergence 2: the data: URI's internal comma stays part of the
    // URL (WHATWG). Firecrawl's split(',') would shred it into phantoms.
    let html = r#"<html><body><img srcset="data:image/avif;base64,AAAA== 1x, /real.jpg 2x"></body></html>"#;
    let got = urls(html, "https://example.com/");
    assert_eq!(
        got,
        vec![
            "data:image/avif;base64,AAAA==".to_string(),
            "https://example.com/real.jpg".to_string(),
        ]
    );
}

#[test]
fn srcset_protocol_relative_token() {
    let html =
        r#"<html><body><img srcset="//cdn.host/a.jpg 1x, //cdn.host/b.jpg 2x"></body></html>"#;
    assert_eq!(
        urls(html, "https://example.com/"),
        vec![
            "https://cdn.host/a.jpg".to_string(),
            "https://cdn.host/b.jpg".to_string(),
        ]
    );
}

#[test]
fn srcset_absolute_and_relative_mixed() {
    let html = r#"<html><body><img srcset="https://cdn.x/a.png 1x, rel/b.png 2x"></body></html>"#;
    assert_eq!(
        urls(html, "https://example.com/dir/"),
        vec![
            "https://cdn.x/a.png".to_string(),
            "https://example.com/dir/rel/b.png".to_string(),
        ]
    );
}
