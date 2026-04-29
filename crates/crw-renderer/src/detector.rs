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
    if html.len() > 80_000 {
        return false;
    }
    let lower = html.to_lowercase();

    // Strong markers: appear ONLY on the interstitial.
    let strong = [
        "cf-browser-verification",
        "cf-challenge-running",
        "/cdn-cgi/challenge-platform/",
        "_cf_chl_opt",
        "__cf_chl_managed_tk__",
        "window._cf_chl_opt",
    ];
    if strong.iter().any(|m| lower.contains(m)) {
        return true;
    }

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
    fn cf_huge_page_not_scanned() {
        let mut html = String::from(r#"<html><body><div id="cf-browser-verification">"#);
        html.push_str(&"<p>x</p>".repeat(20_000));
        html.push_str("</div></body></html>");
        assert!(html.len() > 80_000);
        assert!(!looks_like_cloudflare_challenge(&html));
    }

    #[test]
    fn spinner_class_in_script_body_ignored() {
        // class="spinner" inside a <script> block must not trigger spinner detection,
        // since scripts are stripped before text-length measurement.
        let html = r#"<html><body><article><h1>Real Article</h1><p>This is a real article with substantial content about the topic at hand, providing useful information.</p><script>const x = 'class="spinner"';</script></article></body></html>"#;
        assert!(!looks_like_loading_placeholder(html));
    }
}
