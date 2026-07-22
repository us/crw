use regex::Regex;
use std::sync::LazyLock;

/// Akamai bot-manager reference ID: `Reference #<digits>.<hex>.<digits>.<hex>`.
/// Used only in [`looks_like_vendor_block`] — see the Akamai arm there.
static AKAMAI_REF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"Reference #\d+\.[0-9a-f]+\.\d+\.[0-9a-f]+").expect("static regex")
});

/// Heuristic: does the HTML look like an SPA shell that needs JS rendering?
pub fn needs_js_rendering(html: &str) -> bool {
    // Check up to 500KB — some pages have huge <head> sections (CSS, preloaded data)
    // and the <body> may start well beyond 50KB. Every fixed-cap prefix slice of the
    // page HTML here goes through `floor_char_boundary`: a page longer than the cap
    // can straddle it with a multibyte char, and slicing mid-char panics.
    let was_truncated = html.len() > 500_000;
    let lower = html[..html.floor_char_boundary(500_000)].to_lowercase();
    let body_len = extract_body_text_len(&lower, was_truncated);

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
            "data-reactid",
            "data-remix-run",
            "data-sveltekit",
            "data-astro-",
            "<script src",
            "window.__initial_state__",
            "__next_data__",
            "__nuxt__",
            "__sveltekit_data",
            "window.__remixcontext",
            "window.__astro",
            "gatsby-focus-wrapper",
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

    // Bundler-heavy modern SPA: short body + many script tags. Catches sites
    // that don't expose a recognizable framework marker but ship most of their
    // content via client-side hydration. Threshold is conservative (5+ scripts,
    // body <1000 chars) — 3 scripts is a normal load (analytics + ads + a font
    // loader) on minimal static pages, so we require more before escalating.
    if body_len < 1000 {
        let script_count = lower.matches("<script").count();
        if script_count >= 5 {
            return true;
        }
        let storybook_indicators = [
            "id=\"storybook-root\"",
            "id=\"storybook-docs\"",
            "__storybook",
            "?path=/docs/",
            "/iframe.html",
        ];
        if storybook_indicators.iter().any(|ind| lower.contains(ind)) {
            return true;
        }
    }

    false
}

/// Detect generic anti-bot interstitials (non-Cloudflare): tiny pages whose
/// visible body text consists of a "verifying you're human" / "security check"
/// message. Matched only on visible body text so a JS bundle containing one
/// of these strings cannot false-positive.
pub fn looks_like_generic_bot_wall(html: &str) -> bool {
    if html.len() > 80_000 {
        return false;
    }
    let lower = html.to_lowercase();
    // Most block shells wrap their text in <body>; extracting body-only text
    // keeps a phrase buried in a JS bundle from false-positiving. A few shells
    // (Wikimedia's Varnish error page) omit <body> entirely — the text sits in
    // <div>s directly under <html>, so body-only extraction returns "" and no
    // phrase can ever match. When an HTML document has no <body> tag, fall back
    // to the whole document with <script>/<style> stripped. The fallback is
    // gated on an <html>/<!doctype html> marker so a directly-scraped JSON / XML
    // / plain-text response (which never carries <body>) is NOT scanned — else a
    // small `{"error":"access_denied"}` payload would trip an existing phrase.
    // The 600-char cap below still guards against real articles.
    let body_stripped = if lower.contains("<body") {
        // This fn already bailed above for `html.len() > 80_000`, so the input is
        // never truncated at the 500 KB cap: `was_truncated = false`.
        body_html_without_scripts_lower(&lower, false)
    } else if lower.contains("<html") || lower.contains("<!doctype html") {
        strip_tag_blocks(&strip_tag_blocks(&lower, "script"), "style")
    } else {
        return false;
    };
    let body_text = visible_text_from_stripped_html(&body_stripped);
    if body_text.chars().filter(|c| !c.is_whitespace()).count() > 600 {
        return false;
    }

    let phrases = [
        "performing security verification",
        "verify you are human",
        "checking your browser",
        "enable javascript and cookies",
        "security check",
        "access denied",
        "request blocked",
        // CloudFront / AWS WAF generic block page. Title is the giveaway;
        // body content varies (geo-block, WAF rule, distribution misconfig).
        // All variants render an identical 403 shell with these strings.
        "the request could not be satisfied",
        "generated by cloudfront",
        // Akamai / AWS-style geo-block phrasing — also surfaces on some
        // origin-side firewall pages that don't say "blocked" outright.
        "configured to block access",
        // Wikimedia serves its datacenter-IP ban as an HTTP-200 static error
        // shell (no <body> tag). This canonical footer sentence is unique to
        // that page — a real article never carries it.
        "if you report this error to the wikimedia system administrators",
    ];
    phrases.iter().any(|p| body_text.contains(p))
}

/// Vendor-specific anti-bot block markers. Returns the matched vendor name
/// for logging/metrics, or `None` when no vendor signature is found.
///
/// Markers are curated durable signatures (owned CDN domains, public SDK
/// identifiers, vendor brand strings) — chosen for low false-positive risk
/// and resistance to cosmetic vendor updates. Pair with the catch-all
/// [`looks_like_generic_bot_wall`] phrase list, which handles vendors that
/// haven't been signature-mapped yet.
///
/// Scans only the first 15KB; vendor block pages are small and put their
/// markers in `<head>` or early body. Pages over 200KB return `None` —
/// real content dwarfs vendor block shells.
pub fn looks_like_vendor_block(html: &str) -> Option<&'static str> {
    if html.len() > 200_000 {
        return None;
    }
    let head = &html[..html.floor_char_boundary(15_000)];
    let lower_head = head.to_lowercase();

    // Cloudflare: challenge form with cf-managed token, error code span, or
    // challenge-platform JS loader. All three are unique to CF's anti-bot.
    if (lower_head.contains("challenge-form") && lower_head.contains("__cf_chl_f_tk="))
        || lower_head.contains("cf-error-code")
        || lower_head.contains("/cdn-cgi/challenge-platform/")
    {
        return Some("cloudflare");
    }

    // Akamai: bot-manager reference IDs follow `Reference #<hex>.<hex>.<hex>.<hex>`.
    // "Pardon Our Interruption" is the canonical block page headline.
    if lower_head.contains("pardon our interruption") || AKAMAI_REF_RE.is_match(head) {
        return Some("akamai");
    }

    // PerimeterX: window._pxAppId SDK assignment, owned captcha CDN.
    if lower_head.contains("window._pxappid =") || lower_head.contains("captcha.px-cdn.net") {
        return Some("perimeterx");
    }

    // DataDome: owned captcha delivery domain.
    if lower_head.contains("captcha-delivery.com") {
        return Some("datadome");
    }

    // Imperva / Incapsula: resource marker + incident ID phrasing.
    if lower_head.contains("_incapsula_resource") || lower_head.contains("incapsula incident id") {
        return Some("imperva");
    }

    // Sucuri: WAF block page brand string.
    if lower_head.contains("sucuri website firewall") {
        return Some("sucuri");
    }

    // Kasada: SDK signature.
    if lower_head.contains("kpsdk.scriptstart = kpsdk.now()") {
        return Some("kasada");
    }

    // CloudFront / AWS WAF (geo-block, distribution misconfig). Already
    // partly in the catch-all phrase list — capture as a vendor here for
    // telemetry split.
    if lower_head.contains("generated by cloudfront")
        || lower_head.contains("the request could not be satisfied")
    {
        return Some("cloudfront");
    }

    None
}

/// Returns true when an HTTP response yielded effectively no visible text in
/// the body (post script/style strip). Used by the renderer to decide whether
/// to escalate a "successful" HTTP fetch to JS rendering when no SPA marker
/// was recognized.
///
/// Distinct from [`needs_js_rendering`]: that one is a pre-fetch heuristic
/// on raw markup, looking for framework shells. This one is purely about
/// outcome — does the page have *any* content for an extractor to chew on.
pub fn looks_like_thin_html(html: &str) -> bool {
    let was_truncated = html.len() > 500_000;
    let lower = html[..html.floor_char_boundary(500_000)].to_lowercase();
    extract_body_text_len(&lower, was_truncated) < 200
}

/// Would a headless browser plausibly reveal MORE content than the raw HTTP
/// body? True when the page ships executable JS (an external `<script src=…>`
/// bundle or a non-trivial inline script) OR performs a client-side
/// `<meta http-equiv="refresh">` redirect (which HTTP clients don't follow but
/// a browser does). A thin page with none of these is already complete over
/// HTTP — a headless render reveals nothing and just adds seconds — so the
/// thin-content escalation is gated on this signal.
///
/// Pure-data script blocks (`application/json`, `application/ld+json`,
/// `importmap`, `speculationrules`) never execute, so they do NOT count.
pub fn warrants_browser_retry(html: &str) -> bool {
    let lower = html[..html.floor_char_boundary(500_000)].to_lowercase();

    // Client-side redirect a browser would follow to real content. Matched per
    // <meta> tag (http-equiv refresh + a url target in the SAME tag) so an
    // unrelated string elsewhere can't false-positive.
    for frag in lower.split("<meta").skip(1) {
        let tag = frag.split('>').next().unwrap_or("");
        if tag.contains("http-equiv") && tag.contains("refresh") && tag.contains("url=") {
            return true;
        }
    }

    for frag in lower.split("<script").skip(1) {
        let mut parts = frag.splitn(2, '>');
        let tag = parts.next().unwrap_or("");
        let after = parts.next().unwrap_or("");
        // External bundle → could inject content.
        if tag.contains("src=") {
            return true;
        }
        // Pure-data blocks never execute.
        let is_data = tag.contains("application/json")
            || tag.contains("application/ld+json")
            || tag.contains("importmap")
            || tag.contains("speculationrules");
        if is_data {
            continue;
        }
        // Inline executable script with a non-trivial body.
        let inline = after.split("</script>").next().unwrap_or("");
        if inline.trim().len() > 8 {
            return true;
        }
    }
    false
}

/// Returns true when an extracted markdown is below the floor used by the
/// renderer to decide a fetch produced effectively no extractable content.
/// Pair with [`looks_like_thin_html`] for a full thin-content judgment.
pub fn is_thin_markdown(markdown_len: usize) -> bool {
    markdown_len < 100
}

/// Reason a rendered page is considered a failed render. Returned by
/// [`looks_like_failed_render`] so callers can include the cause in failover
/// warnings or telemetry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailedRenderReason {
    /// Next.js error boundary HTML was injected into the document. Indicates
    /// the framework caught an unhandled exception during hydration or
    /// rendering — the page the user wanted is not present.
    NextJsClientError,
    /// React rendered its production "Minified React error" placeholder. Same
    /// failure class as Next.js, but framework-agnostic.
    ReactMinifiedError,
    /// Next.js root `<div id="__next">` is present but empty (no hydration
    /// took place). Distinct from a generic placeholder because it specifically
    /// indicates an SPA whose JS never executed.
    EmptyNextRoot,
}

impl FailedRenderReason {
    pub fn as_str(self) -> &'static str {
        match self {
            FailedRenderReason::NextJsClientError => "nextjs_client_error",
            FailedRenderReason::ReactMinifiedError => "react_minified_error",
            FailedRenderReason::EmptyNextRoot => "empty_next_root",
        }
    }
}

/// Detect framework-level render failures *in the HTML markup*. Only matches
/// DOM-specific markers (element ids, data attributes) — never visible body
/// text — to avoid false positives on pages that legitimately mention the
/// error string (e.g. a blog post about Next.js debugging).
///
/// Returns `None` when the page looks healthy.
pub fn looks_like_failed_render(html: &str) -> Option<FailedRenderReason> {
    // Bail out fast for very large pages — error boundary markup is small and
    // appears near the body root; scanning megabytes would cost more than it
    // gains. Most failed renders produce <30KB of HTML.
    if html.len() > 200_000 {
        return None;
    }
    let lower = html.to_lowercase();

    // Next.js App Router error boundary. Next renders an error UI with these
    // marker attributes when a client-side exception bubbles to the root.
    // Sources: next.js/packages/next/src/client/components/error-boundary.tsx
    if lower.contains("id=\"__next-error-") || lower.contains("data-nextjs-error") {
        return Some(FailedRenderReason::NextJsClientError);
    }

    // Next.js Pages Router error overlay (visible in dev) and the production
    // "Application error" fallback both render this id. The marker is a DOM
    // element id and only appears when Next chose to render its error path.
    if lower.contains("id=\"__next_error__\"") {
        return Some(FailedRenderReason::NextJsClientError);
    }

    // React production error: surfaces as a minified message with a numeric
    // code linking back to react.dev. Match the canonical anchor href since
    // that combination only exists when React rendered the error explainer.
    if lower.contains("https://react.dev/errors/")
        || lower.contains("https://reactjs.org/docs/error-decoder")
    {
        return Some(FailedRenderReason::ReactMinifiedError);
    }

    // Empty Next.js root shell: <div id="__next"></div> (or with whitespace).
    // The renderer returned the SSR shell but no hydration ran; the
    // user-visible content is missing.
    if let Some(start) = lower.find("id=\"__next\"") {
        let after_id = &lower[start..];
        if let Some(close) = after_id.find('>') {
            let tail = &after_id[close + 1..];
            if let Some(end) = tail.find("</div>") {
                let inner = tail[..end].trim();
                if inner.is_empty() {
                    return Some(FailedRenderReason::EmptyNextRoot);
                }
            }
        }
    }

    None
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
    // Bailed above for `html.len() > 80_000`, so never truncated at the 500 KB cap.
    let body_stripped = body_html_without_scripts_lower(&lower, false);
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
fn body_html_without_scripts_lower(lower: &str, was_truncated: bool) -> String {
    let body_start = lower
        .find("<body")
        .and_then(|i| lower[i..].find('>').map(|j| i + j + 1));
    // A missing `</body>` means two different things. On a page that was cut at
    // the 500 KB scan cap, the tag simply sits past the cut and the real body
    // text is right here in the slice — measuring to the end of the slice is
    // correct (a 1.8 MB article was being called "thin" and needlessly escalated
    // to Chrome). On a NON-truncated page, a missing `</body>` is a genuinely
    // malformed / mid-stream-truncated response, and treating it as thin so it
    // escalates to a fresh render is the right recovery — keep that.
    let body_end = match lower.rfind("</body>") {
        Some(end) => Some(end),
        None if was_truncated => Some(lower.len()),
        None => None,
    };

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
fn extract_body_text_len(lower: &str, was_truncated: bool) -> usize {
    if !lower.contains("<body") {
        return 1000;
    }
    let stripped = body_html_without_scripts_lower(lower, was_truncated);
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

// ── Cloudflare challenge detection ───────────────────────────────────

/// Detect a Cloudflare anti-bot challenge / interstitial in the response.
///
/// Strategy: a single weak marker is not enough — most marketing pages
/// reference Cloudflare somewhere. We require either a *strong* marker
/// (uniquely tied to the challenge interstitial) or a *combination* of
/// two weak markers. This keeps false positives low while still catching
/// the JS-challenge HTML LightPanda fails to solve.
///
/// Pair with [`is_cloudflare_mitigated_header`] which uses the
/// `cf-mitigated` response header — that signal is independent of body
/// content and is the most reliable indicator.
pub fn looks_like_cloudflare_challenge(html: &str) -> bool {
    // Strong markers appear ONLY on the interstitial and can sit deep in the
    // body of a large managed-challenge page — measured at byte ~128k of a 275k
    // Glassdoor "Just a moment" page, inside a
    // `<script src="/cdn-cgi/challenge-platform/…/orchestrate/…">`. So scan them
    // regardless of the 80KB weak-marker cap, bounded to the first 512KB. The
    // markers are fixed-lowercase ASCII CF tokens, so match case-sensitively on
    // the raw bytes (no allocation on the hot per-attempt path; mirrors
    // `crw_crawl::single::classify_block`).
    const STRONG_SCAN_LIMIT: usize = 512 * 1024;
    let strong_src = &html[..html.floor_char_boundary(STRONG_SCAN_LIMIT)];
    const STRONG: [&str; 5] = [
        "cf-browser-verification",
        "cf-challenge-running",
        "/cdn-cgi/challenge-platform/",
        "_cf_chl_opt", // substring of window._cf_chl_opt / __cf_chl_managed_tk__
        "__cf_chl_managed_tk__",
    ];
    if STRONG.iter().any(|m| strong_src.contains(m)) {
        return true;
    }

    // Weak markers can appear on legitimate Cloudflare-fronted pages, so they
    // keep the size guard: a large page is real content, not an interstitial.
    if html.len() > 80_000 {
        return false;
    }
    let lower = html.to_lowercase();

    // Weak markers: each can appear on legitimate Cloudflare-fronted pages.
    // "ray id:" + "cloudflare" co-occur on most CF-fronted error pages and
    // would false-positive a real page with a CF footer; require challenge-
    // specific phrasing instead.
    let weak = [
        "just a moment",
        "checking your browser",
        "attention required",
        "performance &amp; security by cloudflare",
        "performance & security by cloudflare",
    ];
    let weak_hits = weak.iter().filter(|m| lower.contains(*m)).count();
    weak_hits >= 2
}

/// Returns true when the `cf-mitigated` response header indicates the
/// request was challenged or blocked by Cloudflare. Independent of the
/// HTTP status code — Cloudflare may return 200 with this header set.
///
/// `header_value` is the raw header value (case-sensitive on the right
/// side; we lower-case here for safety).
pub fn is_cloudflare_mitigated_header(header_value: &str) -> bool {
    let lower = header_value.trim().to_ascii_lowercase();
    matches!(lower.as_str(), "challenge" | "block")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thin_static_page_does_not_warrant_browser() {
        // example.com: a genuinely small static doc, zero scripts. Thin, but a
        // browser render would reveal nothing — must NOT be treated as needing JS.
        let html = "<html><head><title>Example Domain</title></head><body>\
            <div><h1>Example Domain</h1><p>This domain is for use in illustrative \
            examples.</p></div></body></html>";
        assert!(looks_like_thin_html(html));
        assert!(!warrants_browser_retry(html));
    }

    #[test]
    fn thin_shell_with_script_bundle_warrants_browser() {
        // Unrecognized-shell case (espn/seattletimes bucket): thin body but ships
        // a JS bundle → still escalates.
        let html =
            r#"<html><body><div id="app"></div><script src="/bundle.js"></script></body></html>"#;
        assert!(looks_like_thin_html(html));
        assert!(warrants_browser_retry(html));
    }

    #[test]
    fn inline_executable_script_warrants_browser() {
        let html =
            r#"<html><body><div></div><script>window.__DATA__={};main();</script></body></html>"#;
        assert!(warrants_browser_retry(html));
    }

    #[test]
    fn json_ld_only_does_not_warrant_browser() {
        // Structured-data blocks don't execute — a static page carrying only
        // JSON-LD must not be pushed to a browser.
        let html = r#"<html><body><p>hi</p><script type="application/ld+json">{"@type":"Thing"}</script></body></html>"#;
        assert!(!warrants_browser_retry(html));
    }

    #[test]
    fn thin_meta_refresh_redirect_warrants_browser() {
        // A thin stub whose only mechanism is a client-side meta-refresh redirect
        // (no script). HTTP clients don't follow it; a browser would, then reach
        // real content — so escalate.
        let html = r#"<html><head><meta http-equiv="refresh" content="0; url=https://example.org/real"></head><body>Redirecting...</body></html>"#;
        assert!(looks_like_thin_html(html));
        assert!(warrants_browser_retry(html));
    }

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
    fn detects_nextjs_app_router_error_boundary() {
        let html = r#"<html><body><div id="__next-error-0"><h2>Application error: a client-side exception has occurred.</h2></div></body></html>"#;
        assert_eq!(
            looks_like_failed_render(html),
            Some(FailedRenderReason::NextJsClientError)
        );
    }

    #[test]
    fn detects_nextjs_pages_router_error() {
        let html = r#"<html><body><div id="__next_error__">oops</div></body></html>"#;
        assert_eq!(
            looks_like_failed_render(html),
            Some(FailedRenderReason::NextJsClientError)
        );
    }

    #[test]
    fn detects_react_minified_error() {
        let html = r#"<html><body><a href="https://react.dev/errors/418">Minified React error #418</a></body></html>"#;
        assert_eq!(
            looks_like_failed_render(html),
            Some(FailedRenderReason::ReactMinifiedError)
        );
    }

    #[test]
    fn detects_legacy_react_error_decoder_url() {
        let html = r#"<html><body><a href="https://reactjs.org/docs/error-decoder.html?invariant=31">React</a></body></html>"#;
        assert_eq!(
            looks_like_failed_render(html),
            Some(FailedRenderReason::ReactMinifiedError)
        );
    }

    #[test]
    fn blog_post_about_error_is_not_failed_render() {
        // Regression: a blog post that *describes* the Next.js error must NOT
        // be flagged as a failed render. The string appears only in body text
        // (and not in a __next-error- element id), so the detector must let
        // it through.
        let html = r#"<html><body><article><h1>Debugging Next.js</h1>
            <p>When you see "Application error: a client-side exception has occurred",
            it usually means a hydration mismatch.</p>
            <pre><code>console.log('debug')</code></pre>
        </article></body></html>"#;
        assert!(looks_like_failed_render(html).is_none());
    }

    #[test]
    fn healthy_page_is_not_failed_render() {
        let html =
            r#"<html><body><main><h1>Hello</h1><p>Real content here.</p></main></body></html>"#;
        assert!(looks_like_failed_render(html).is_none());
    }

    #[test]
    fn huge_page_is_not_scanned() {
        // Pages over 200KB are exempt. Even with a marker that would normally
        // trigger, the function must short-circuit to None.
        let mut html = String::from(r#"<html><body><div id="__next-error-0"></div>"#);
        html.push_str(&"<p>filler</p>".repeat(20_000));
        html.push_str("</body></html>");
        assert!(html.len() > 200_000);
        assert!(looks_like_failed_render(&html).is_none());
    }

    #[test]
    fn cf_strong_marker_detected() {
        let html =
            r#"<html><body><div id="cf-browser-verification">Just a moment...</div></body></html>"#;
        assert!(looks_like_cloudflare_challenge(html));
    }

    #[test]
    fn cf_managed_token_detected() {
        let html = r#"<html><body><script>window._cf_chl_opt={cvId:'2'};</script></body></html>"#;
        assert!(looks_like_cloudflare_challenge(html));
    }

    #[test]
    fn cf_strong_marker_detected_on_large_page() {
        // Modern Cloudflare managed challenge: a large (>80KB) HTML whose only
        // machine marker is a challenge-platform <script src> deep in the body
        // (Glassdoor served a 275KB page with the marker at byte ~128k). The old
        // 80KB size cap made this evade detection.
        let mut html = String::from("<html><head><title>Just a moment...</title></head><body>");
        html.push_str(&"<p>verifying you are human, one moment please.</p>".repeat(3_000));
        html.push_str(
            r#"<script src="/cdn-cgi/challenge-platform/h/b/orchestrate/chl_page/v1?ray=abc"></script>"#,
        );
        html.push_str("</body></html>");
        assert!(
            html.len() > 80_000,
            "fixture must exceed the weak-marker cap"
        );
        assert!(looks_like_cloudflare_challenge(&html));
    }

    #[test]
    fn cf_large_real_page_with_footer_mention_not_flagged() {
        // A large real page that merely mentions Cloudflare in a footer (no
        // challenge markers) must NOT be flagged — the weak-marker path stays
        // size-guarded.
        let mut html = String::from("<html><body><article>");
        html.push_str(&"<p>Real article content about web performance.</p>".repeat(3_000));
        html.push_str(
            "<footer>Hosted via Cloudflare. Ray ID: abc123</footer></article></body></html>",
        );
        assert!(html.len() > 80_000);
        assert!(!looks_like_cloudflare_challenge(&html));
    }

    #[test]
    fn cf_single_weak_marker_not_enough() {
        // A page that just mentions "Cloudflare" should not trigger.
        let html = r#"<html><body><article><h1>Why we use Cloudflare</h1><p>Performance benefits.</p></article></body></html>"#;
        assert!(!looks_like_cloudflare_challenge(html));
    }

    #[test]
    fn cf_two_weak_markers_trigger() {
        // Two challenge-specific phrases must co-occur. "ray id:" was
        // removed from the weak set because legitimate CF-fronted error
        // pages also include both "ray id" and "cloudflare".
        let html =
            r#"<html><body><h1>Just a moment...</h1><p>Checking your browser...</p></body></html>"#;
        assert!(looks_like_cloudflare_challenge(html));
    }

    #[test]
    fn cf_ray_id_alone_does_not_trigger() {
        // Pre-fix this would false-positive: "ray id" + "cloudflare" both
        // appear on benign CF-fronted pages.
        let html = r#"<html><body><h1>About</h1><p>Hosted via Cloudflare.</p><footer>Ray ID: abc123</footer></body></html>"#;
        assert!(!looks_like_cloudflare_challenge(html));
    }

    #[test]
    fn cf_mitigated_header_challenge() {
        assert!(is_cloudflare_mitigated_header("challenge"));
        assert!(is_cloudflare_mitigated_header(" CHALLENGE "));
        assert!(is_cloudflare_mitigated_header("block"));
    }

    #[test]
    fn cf_mitigated_header_other_values() {
        assert!(!is_cloudflare_mitigated_header(""));
        assert!(!is_cloudflare_mitigated_header("ok"));
        assert!(!is_cloudflare_mitigated_header("verified"));
    }

    #[test]
    fn cf_strong_marker_beyond_scan_limit_not_flagged() {
        // Perf bound: strong markers are scanned only within the first 512KB.
        // A marker past that (pathologically large page) is not scanned — real
        // CF challenge markers sit well within the first few hundred KB.
        let mut html = String::from("<html><body>");
        html.push_str(&"<p>x</p>".repeat(80_000)); // ~640KB of filler > 512KB
        html.push_str(r#"<div id="cf-browser-verification"></div></body></html>"#);
        assert!(html.len() > 512 * 1024);
        assert!(!looks_like_cloudflare_challenge(&html));
    }

    #[test]
    fn cloudfront_403_block_page_is_bot_wall() {
        // Real-world CloudFront geo-block page (americastire.com from EU egress).
        // Two strong markers: title and footer attribution. Engine must escalate
        // through the renderer chain (e.g. to chrome_proxy) instead of returning
        // this 403 shell as success.
        let html = r#"<html><head><title>ERROR: The request could not be satisfied</title></head>
            <body><h1>403 ERROR</h1>
            <h3>The request could not be satisfied.</h3>
            <p>The Amazon CloudFront distribution is configured to block access from your country.</p>
            <hr><i>Generated by cloudfront (CloudFront)</i></body></html>"#;
        assert!(looks_like_generic_bot_wall(html));
    }

    #[test]
    fn generic_403_with_block_phrasing_is_bot_wall() {
        // Origin-side / WAF block page that uses "configured to block access"
        // without naming a vendor — still a clear block signal.
        let html = r#"<html><body><h1>403</h1>
            <p>Our firewall is configured to block access from this region.</p></body></html>"#;
        assert!(looks_like_generic_bot_wall(html));
    }

    /// The Wikimedia Varnish error shell, reproducing the real structure: an
    /// HTTP-200 static page with NO <body>/<head> tags, one inline <style>, and
    /// the content in <div>s directly under <html> (source: Wikimedia
    /// operations/puppet error templates).
    fn wikimedia_block_html() -> &'static str {
        r#"<!DOCTYPE html>
<html lang="en">
<meta charset="utf-8">
<title>Wikimedia Error</title>
<style>body{font-family:sans-serif}</style>
<meta name="color-scheme" content="light dark">
<div class="content" role="main">
<h1>Error</h1>
<p>Contabo networks are forbidden due to abuse. Contact noc@wikimedia.org for assistance.</p>
</div>
<div class="footer">
<p>If you report this error to the Wikimedia System Administrators, please include the details below.</p>
<p class="text-muted"><code>Request served via cp6016, Varnish XID 12345<br>Error: 403, Contabo networks are forbidden due to abuse.<br><details><summary>Sensitive client information</summary>IP address: 207.180.230.151</details></code></p>
</div>
</html>"#
    }

    #[test]
    fn wikimedia_http200_block_shell_is_bot_wall() {
        // Regression for the silent-success bug: the Wikimedia error page is
        // HTTP-200, scriptless, and crucially has NO <body> tag. Body-only text
        // extraction returned "" here, so the phrase list never matched. The
        // no-<body> fallback must let the canonical footer phrase trip.
        assert!(looks_like_generic_bot_wall(wikimedia_block_html()));
    }

    #[test]
    fn json_api_error_without_body_is_not_bot_wall() {
        // Regression: a directly-scraped JSON/plain-text error response has no
        // <body> AND no <html> marker, so the no-<body> fallback must not scan
        // it — otherwise the existing "access denied" phrase would wrongly flag
        // a legitimate small API payload as a block.
        let json = r#"{"error":"access_denied","message":"Access Denied: insufficient permissions for this resource"}"#;
        assert!(!looks_like_generic_bot_wall(json));
        let xml = r#"<?xml version="1.0"?><Error><Code>AccessDenied</Code><Message>Access Denied</Message></Error>"#;
        assert!(!looks_like_generic_bot_wall(xml));
    }

    #[test]
    fn real_wikipedia_article_is_not_bot_wall() {
        // A real article (>600 visible chars, normal <body>) must NOT trip.
        let html = format!(
            "<html><body><article><h1>Radcliffe College</h1>{}</article></body></html>",
            "<p>Radcliffe College was a women's liberal arts college in Cambridge, \
             Massachusetts, and functioned as the female coordinate institution for \
             the all-male Harvard College.</p>"
                .repeat(6)
        );
        assert!(!looks_like_generic_bot_wall(&html));
    }

    #[test]
    fn legitimate_blog_about_cloudfront_is_not_bot_wall() {
        // Regression guard: a long article about CloudFront must NOT trip the
        // bot-wall heuristic. visible body text length cap (600 chars) is the
        // existing safeguard — exceed it here to assert it still applies.
        let mut html = String::from(r#"<html><body><article>"#);
        html.push_str("<p>An article about CloudFront and how distributions are configured to block access by country. </p>".repeat(20).as_str());
        html.push_str("</article></body></html>");
        assert!(!looks_like_generic_bot_wall(&html));
    }

    #[test]
    fn vendor_cloudflare_challenge_form_detected() {
        let html = r#"<html><body><form class="challenge-form" action="/?__cf_chl_f_tk=abc123">
            </form></body></html>"#;
        assert_eq!(looks_like_vendor_block(html), Some("cloudflare"));
    }

    #[test]
    fn vendor_cloudflare_error_code_detected() {
        let html = r#"<html><body><span class="cf-error-code">1020</span></body></html>"#;
        assert_eq!(looks_like_vendor_block(html), Some("cloudflare"));
    }

    #[test]
    fn vendor_cloudflare_challenge_platform_detected() {
        let html = r#"<html><head><script src="/cdn-cgi/challenge-platform/h/g/orchestrate/chl_page/v1?ray=abc"></script></head></html>"#;
        assert_eq!(looks_like_vendor_block(html), Some("cloudflare"));
    }

    #[test]
    fn vendor_akamai_reference_id_detected() {
        let html = r#"<html><body><p>Access Denied</p>
            <p>Reference #18.2d351ab8.1557333295.a4e16ab</p></body></html>"#;
        assert_eq!(looks_like_vendor_block(html), Some("akamai"));
    }

    #[test]
    fn vendor_akamai_pardon_our_interruption_detected() {
        let html = r#"<html><body><h1>Pardon Our Interruption</h1>
            <p>As you were browsing, something about your browser made us think you were a bot.</p>
            </body></html>"#;
        assert_eq!(looks_like_vendor_block(html), Some("akamai"));
    }

    #[test]
    fn vendor_perimeterx_pxappid_detected() {
        let html = r#"<html><head><script>window._pxAppId = 'PXabc123';</script></head></html>"#;
        assert_eq!(looks_like_vendor_block(html), Some("perimeterx"));
    }

    #[test]
    fn vendor_datadome_captcha_domain_detected() {
        let html = r#"<html><body><iframe src="https://geo.captcha-delivery.com/captcha/?initialCid=xyz"></iframe></body></html>"#;
        assert_eq!(looks_like_vendor_block(html), Some("datadome"));
    }

    #[test]
    fn vendor_imperva_incapsula_resource_detected() {
        let html = r#"<html><body><script src="/_Incapsula_Resource?SWJIYLWA=blah"></script></body></html>"#;
        assert_eq!(looks_like_vendor_block(html), Some("imperva"));
    }

    #[test]
    fn vendor_sucuri_firewall_brand_detected() {
        let html = r#"<html><body><h1>Sucuri WebSite Firewall - Access Denied</h1></body></html>"#;
        assert_eq!(looks_like_vendor_block(html), Some("sucuri"));
    }

    #[test]
    fn vendor_cloudfront_block_detected() {
        let html = r#"<html><head><title>ERROR: The request could not be satisfied</title></head>
            <body><h1>403 ERROR</h1>
            <hr><i>Generated by cloudfront (CloudFront)</i></body></html>"#;
        assert_eq!(looks_like_vendor_block(html), Some("cloudfront"));
    }

    #[test]
    fn vendor_legit_blog_about_cloudflare_is_none() {
        // A 50KB legit page mentioning Cloudflare but with no challenge markers
        // must NOT be flagged as a vendor block.
        let mut html = String::from("<html><body><article><h1>Why we picked Cloudflare</h1>");
        html.push_str(
            &"<p>Cloudflare gives us DDoS protection and a global anycast network.</p>".repeat(400),
        );
        html.push_str("</article></body></html>");
        assert!(html.len() > 15_000);
        assert!(looks_like_vendor_block(&html).is_none());
    }

    #[test]
    fn vendor_block_oversized_page_returns_none() {
        let big = "x".repeat(300_000);
        assert!(looks_like_vendor_block(&big).is_none());
    }

    #[test]
    fn vendor_block_clean_page_returns_none() {
        let html = r#"<html><body><main><h1>Hello</h1><p>Real content.</p></main></body></html>"#;
        assert!(looks_like_vendor_block(html).is_none());
    }

    #[test]
    fn spinner_class_in_script_body_ignored() {
        // class="spinner" inside a <script> block must not trigger spinner detection,
        // since scripts are stripped before text-length measurement.
        let html = r#"<html><body><article><h1>Real Article</h1><p>This is a real article with substantial content about the topic at hand, providing useful information.</p><script>const x = 'class="spinner"';</script></article></body></html>"#;
        assert!(!looks_like_loading_placeholder(html));
    }

    /// Pad `head` with ASCII so the multibyte `c` straddles byte `cap`, landing
    /// `offset` bytes into it — a mid-char index a naive `&html[..cap]` panics on.
    /// `tail` closes the page past the cap.
    fn page_with_char_straddling(
        head: &str,
        cap: usize,
        tail: &str,
        c: char,
        offset: usize,
    ) -> String {
        let mut html = String::with_capacity(cap + tail.len() + 8);
        html.push_str(head);
        html.push_str(&"a".repeat(cap - offset - head.len()));
        html.push(c);
        html.push_str(tail);
        assert!(!html.is_char_boundary(cap), "byte {cap} must be mid-char");
        assert!(html.len() > cap, "page must exceed the cap");
        html
    }

    /// Every fixed-cap prefix scan in this module, paired with a page that
    /// straddles that cap with a multibyte char and the verdict a correct scan
    /// must still reach. Slicing mid-char panics outright; a clamp that walks
    /// back too far scans the wrong window and flips the verdict instead. Each
    /// verdict is false on an empty window, so none of these pass vacuously.
    #[test]
    fn multibyte_char_straddling_a_scan_cap_keeps_the_verdict() {
        type Verdict = fn(&str) -> bool;
        let cases: [(&str, usize, &str, Verdict); 3] = [
            // 500KB SPA/thin scan: an SPA marker for `needs_js_rendering`, and an
            // inline script for `warrants_browser_retry` (drop it and that arm
            // goes false). The body reads as empty because the cap truncates ahead
            // of `</body>`, not because the script is stripped — the padding is
            // what pushes the close tag out of the scanned window.
            (
                r#"<html><body><div id="root"></div><script>x="#,
                500_000,
                "\"</script></body></html>",
                |h| needs_js_rendering(h) && looks_like_thin_html(h) && warrants_browser_retry(h),
            ),
            // 15KB vendor-block head scan, kept under the 200KB vendor ceiling.
            (
                r#"<html><head><script src="/cdn-cgi/challenge-platform/h/x"></script>"#,
                15_000,
                "</head></html>",
                |h| looks_like_vendor_block(h) == Some("cloudflare"),
            ),
            // 512KB strong-marker scan: a managed-challenge page big enough to
            // reach the cap, well past the 80KB weak-marker guard.
            (
                r#"<html><body><script src="/cdn-cgi/challenge-platform/h/b/orchestrate/j"></script>"#,
                512 * 1024,
                "</body></html>",
                looks_like_cloudflare_challenge,
            ),
        ];

        for (head, cap, tail, verdict) in cases {
            // 'é' can only ever be split one byte in; the 4-byte emoji is the
            // widest char and straddles at three distinct interior offsets, so a
            // clamp that clears a single byte still splits it.
            for c in ['é', '\u{1F600}'] {
                for offset in 1..c.len_utf8() {
                    let html = page_with_char_straddling(head, cap, tail, c, offset);
                    assert!(
                        verdict(&html),
                        "cap {cap}, char {c:?}, straddle offset {offset}"
                    );
                }
            }
        }
    }
}
