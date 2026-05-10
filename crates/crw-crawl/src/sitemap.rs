use crw_core::error::CrwResult;
use scraper::{Html, Selector};

/// Hard cap on a single sitemap response body. The sitemap spec (sitemaps.org,
/// 2017 revision) raised the per-file limit to 50 MB uncompressed / 50,000
/// URLs, so anything below that risks silently dropping legit sitemaps from
/// large sites. We match the spec ceiling.
const MAX_SITEMAP_BYTES: usize = 50 * 1024 * 1024; // 50 MB

/// Per-fetch timeout for sitemap GET/HEAD. Without this a single slow sitemap
/// can starve the whole map operation budget.
const SITEMAP_FETCH_TIMEOUT_SECS: u64 = 15;

/// Hard ceiling on parallel sitemap GETs against a single host, regardless of
/// the configured `max_concurrency`. Sitemap responses are small and the
/// dominant cost is round-trip latency, so a small ceiling captures most of
/// the speedup without flooding the target.
const SITEMAP_FETCH_CONCURRENCY_CAP: usize = 8;

/// Result of parsing a sitemap document.
///
/// Sitemap-index entries (`<sitemap><loc>`) and urlset entries (`<url><loc>`)
/// are kept separate so callers can recurse into indexes without confusing
/// child sitemap URLs for page URLs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SitemapResult {
    pub page_urls: Vec<String>,
    pub child_sitemaps: Vec<String>,
}

impl SitemapResult {
    pub fn is_empty(&self) -> bool {
        self.page_urls.is_empty() && self.child_sitemaps.is_empty()
    }
}

/// Fetch a single sitemap and return parsed result. Returns empty on any
/// failure (timeout, non-2xx, oversized body, gzip error, HTML masquerade,
/// cross-origin redirect).
///
/// A cross-origin redirect (e.g. `example.com/sitemap.xml` → `evil.com/sm.xml`)
/// is rejected even though the redirect target is SSRF-safe, because the
/// sitemap-tree contract is "stay within the target origin". Without this
/// check, a same-origin seed could 302 us into parsing an attacker-controlled
/// sitemap body. We compare full origin tuples (scheme+host+port).
///
/// Errors are logged but not surfaced — callers iterate over multiple seeds
/// and a single bad sitemap should not abort discovery.
pub async fn fetch_sitemap(url: &str, client: &reqwest::Client) -> CrwResult<SitemapResult> {
    let requested_origin = match url::Url::parse(url).ok().as_ref().and_then(origin_key) {
        Some(k) => k,
        None => {
            tracing::debug!("sitemap fetch skipped: cannot parse origin for {url}");
            return Ok(SitemapResult::default());
        }
    };

    let resp = match client
        .get(url)
        .timeout(std::time::Duration::from_secs(SITEMAP_FETCH_TIMEOUT_SECS))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!("sitemap fetch error for {url}: {e}");
            return Ok(SitemapResult::default());
        }
    };

    let final_url = resp.url().clone();
    if final_url.as_str() != url {
        tracing::debug!("sitemap redirect: {} -> {}", url, final_url);
    }
    match origin_key(&final_url) {
        Some(ref k) if k == &requested_origin => {}
        _ => {
            tracing::warn!("sitemap {url} redirected cross-origin to {final_url}, dropping");
            return Ok(SitemapResult::default());
        }
    }
    if !resp.status().is_success() {
        tracing::debug!("sitemap {url} returned {}", resp.status());
        return Ok(SitemapResult::default());
    }

    let bytes = match read_body_capped(resp, MAX_SITEMAP_BYTES).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("sitemap body read failed for {url}: {e}");
            return Ok(SitemapResult::default());
        }
    };

    // Magic bytes are authoritative. Reqwest's `gzip` feature already
    // transparently decodes responses with `Content-Encoding: gzip`, so a
    // `.xml.gz` URL served with that header arrives here as plain XML — the
    // magic bytes will be absent and we skip the decode. We still handle the
    // case where a server returns the raw gzip stream as the body without
    // Content-Encoding (some CDNs do this for `.gz` files).
    let magic_says_gz = bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b;
    let xml_bytes = if magic_says_gz {
        match decode_gzip_capped(&bytes, MAX_SITEMAP_BYTES) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("sitemap gzip decode failed for {url}: {e}");
                return Ok(SitemapResult::default());
            }
        }
    } else {
        bytes
    };

    let text = String::from_utf8_lossy(&xml_bytes);
    Ok(parse_sitemap_with_sniff(&text))
}

/// Issue a HEAD request — used by the discover layer to skip body GETs on
/// fallback sitemap paths that are clearly 404. Returns false on any error
/// (HEAD often unsupported / cached as 4xx by CDNs).
pub async fn head_probe(url: &str, client: &reqwest::Client) -> bool {
    match client
        .head(url)
        .timeout(std::time::Duration::from_secs(SITEMAP_FETCH_TIMEOUT_SECS))
        .send()
        .await
    {
        Ok(r) => r.status().is_success(),
        Err(_) => false,
    }
}

/// Stream the body and abort if it would exceed `max` bytes.
async fn read_body_capped(resp: reqwest::Response, max: usize) -> Result<Vec<u8>, String> {
    use futures::stream::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        if buf.len() + chunk.len() > max {
            return Err(format!("body exceeds {max} bytes cap"));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// Decode gzip with the same byte cap as the compressed body. A 10 MB gzip
/// payload can otherwise decompress to many GB ("gzip bomb"); without this
/// cap a single crafted sitemap.xml.gz would OOM the engine.
fn decode_gzip_capped(data: &[u8], max: usize) -> Result<Vec<u8>, String> {
    use flate2::read::GzDecoder;
    use std::io::Read;
    let dec = GzDecoder::new(data);
    // `take(max + 1)` lets us detect overflow: if read_to_end fills exactly
    // `max + 1` bytes the decoded payload is over the cap.
    let mut limited = dec.take((max as u64) + 1);
    let mut out = Vec::new();
    limited.read_to_end(&mut out).map_err(|e| e.to_string())?;
    if out.len() > max {
        return Err(format!("decoded body exceeds {max} bytes cap"));
    }
    Ok(out)
}

/// Detect HTML masquerading as sitemap (Cloudflare challenge, soft 404 pages
/// served with HTTP 200). Returns empty if detected, else parses normally.
fn parse_sitemap_with_sniff(xml: &str) -> SitemapResult {
    let trimmed = xml.trim_start();
    // Walk back to the nearest UTF-8 char boundary so a 2048th-byte split in
    // the middle of a multi-byte sequence doesn't panic. `is_char_boundary`
    // is true at index 0 and at index `len`, so this loop always terminates.
    let mut head_len = trimmed.len().min(2048);
    while !trimmed.is_char_boundary(head_len) {
        head_len -= 1;
    }
    let head = trimmed[..head_len].to_lowercase();
    if head.starts_with("<!doctype html")
        || head.starts_with("<html")
        || head.contains("just a moment")
        || head.contains("cf-mitigated")
        || head.contains("cf-chl-")
    {
        tracing::warn!("sitemap response looks like HTML, not XML; ignoring");
        return SitemapResult::default();
    }
    parse_sitemap(xml)
}

/// Parse sitemap XML and split entries by kind.
///
/// `<url><loc>` → page URLs.
/// `<sitemap><loc>` → child sitemap URLs (a `<sitemapindex>` document).
/// Bare `<loc>` outside either parent is treated as a page URL fallback,
/// only used when no other matches were found.
pub fn parse_sitemap(xml: &str) -> SitemapResult {
    let document = Html::parse_document(xml);
    let mut result = SitemapResult::default();

    if let Ok(sel) = Selector::parse("url > loc") {
        for el in document.select(&sel) {
            let text: String = el.text().collect();
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                result.page_urls.push(trimmed.to_string());
            }
        }
    }

    if let Ok(sel) = Selector::parse("sitemap > loc") {
        for el in document.select(&sel) {
            let text: String = el.text().collect();
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                result.child_sitemaps.push(trimmed.to_string());
            }
        }
    }

    if result.page_urls.is_empty()
        && result.child_sitemaps.is_empty()
        && let Ok(sel) = Selector::parse("loc")
    {
        for el in document.select(&sel) {
            let text: String = el.text().collect();
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                result.page_urls.push(trimmed.to_string());
            }
        }
    }

    result
}

/// BFS over a sitemap tree. Same-origin filter applies to both child sitemaps
/// and page URLs to prevent the engine from being abused as a sitemap-fetch
/// proxy (a crafted index could otherwise point us at arbitrary public hosts).
///
/// Each BFS level is fetched in parallel via `buffer_unordered(max_concurrency)`
/// — sequential fetching of a 10-child WordPress index would otherwise add
/// roughly 9× latency to discovery, but unbounded `join_all` would also burst
/// up to `max_sitemaps` concurrent same-host requests, ignoring the operator's
/// politeness setting.
pub async fn fetch_sitemap_tree(
    seeds: Vec<String>,
    target_origin: &url::Url,
    client: &reqwest::Client,
    max_depth: u32,
    max_sitemaps: usize,
    max_urls: usize,
    max_concurrency: usize,
) -> Vec<String> {
    use futures::stream::{self, StreamExt};
    use std::collections::HashSet;

    let target_key = match origin_key(target_origin) {
        Some(k) => k,
        None => return Vec::new(),
    };
    // Sitemap fetches are light (10 MB cap, 15 s timeout), but we still bound
    // them so a sitemap-index with N children never exceeds the configured
    // crawler concurrency. 1 disables the WordPress speedup entirely; cap at
    // a small floor of 2 so politeness=1 still recurses without parallel hits.
    let concurrency = max_concurrency.clamp(1, SITEMAP_FETCH_CONCURRENCY_CAP);

    let mut visited: HashSet<String> = HashSet::new();
    let mut all_pages: HashSet<String> = HashSet::new();
    let mut current: Vec<String> = seeds
        .into_iter()
        .filter(|u| same_origin_url(u, &target_key))
        .collect();
    let mut depth: u32 = 0;
    let mut total_fetched: usize = 0;

    while !current.is_empty() && depth <= max_depth && total_fetched < max_sitemaps {
        let remaining_budget = max_sitemaps.saturating_sub(total_fetched);
        let batch: Vec<String> = current
            .drain(..)
            .filter(|u| visited.insert(u.clone()))
            .take(remaining_budget)
            .collect();

        if batch.is_empty() {
            break;
        }
        total_fetched += batch.len();

        let results: Vec<(String, SitemapResult)> = stream::iter(batch)
            .map(|u| {
                let client = client.clone();
                async move {
                    let res = fetch_sitemap(&u, &client).await.unwrap_or_default();
                    (u, res)
                }
            })
            .buffer_unordered(concurrency)
            .collect()
            .await;

        let mut next: Vec<String> = Vec::new();
        for (_parent, res) in results {
            for page in res.page_urls {
                if all_pages.len() >= max_urls {
                    break;
                }
                if same_origin_url(&page, &target_key) {
                    all_pages.insert(page);
                }
            }
            for child in res.child_sitemaps {
                if same_origin_url(&child, &target_key) && !visited.contains(&child) {
                    next.push(child);
                }
            }
        }

        if all_pages.len() >= max_urls {
            tracing::info!(
                cap = max_urls,
                "sitemap_tree hit max_urls cap, stopping discovery"
            );
            break;
        }
        current = next;
        depth += 1;
    }
    if total_fetched >= max_sitemaps {
        tracing::info!(
            cap = max_sitemaps,
            fetched = total_fetched,
            "sitemap_tree hit max_sitemaps cap"
        );
    }

    all_pages.into_iter().collect()
}

/// Origin tuple: scheme + lowercased host + effective port.
/// Two URLs match iff all three components match. `http://x` and `https://x`
/// do not cross over, neither do `:80` vs `:8080`. Subdomains stay distinct
/// (`cdn.x.com` ≠ `x.com`, and crucially `www.x.com` ≠ `x.com`). The strict
/// host comparison is the load-bearing security guarantee for the redirect
/// guard and the sitemap-tree filter — relaxing it (e.g. apex/www equivalence)
/// would let a redirect or child-sitemap entry escape the requested origin.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OriginKey {
    scheme: String,
    host: String,
    port: u16,
}

fn origin_key(u: &url::Url) -> Option<OriginKey> {
    let scheme = u.scheme();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let host = u.host_str()?.to_lowercase();
    let port = u.port_or_known_default()?;
    Some(OriginKey {
        scheme: scheme.to_string(),
        host,
        port,
    })
}

fn same_origin_url(u: &str, target: &OriginKey) -> bool {
    // SSRF safety is enforced at the route entry (validate_safe_url) and at
    // the reqwest client level (safe_redirect_policy). Inside the sitemap
    // tree the security boundary is the origin match — a child URL must not
    // escape the (scheme, host, port) we were asked to map.
    let parsed = match url::Url::parse(u) {
        Ok(p) => p,
        Err(_) => return false,
    };
    match origin_key(&parsed) {
        Some(k) => &k == target,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_urlset_into_page_urls() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url><loc>https://example.com/page1</loc></url>
  <url><loc>https://example.com/page2</loc></url>
</urlset>"#;
        let r = parse_sitemap(xml);
        assert_eq!(r.page_urls.len(), 2);
        assert!(r.child_sitemaps.is_empty());
    }

    #[test]
    fn parses_sitemap_index_into_child_sitemaps() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <sitemap><loc>https://example.com/wp-sitemap-posts-1.xml</loc></sitemap>
  <sitemap><loc>https://example.com/wp-sitemap-posts-2.xml</loc></sitemap>
</sitemapindex>"#;
        let r = parse_sitemap(xml);
        assert!(r.page_urls.is_empty());
        assert_eq!(r.child_sitemaps.len(), 2);
    }

    #[test]
    fn html_pretending_to_be_sitemap_returns_empty() {
        let html = "<!DOCTYPE html><html><body>Not Found</body></html>";
        let r = parse_sitemap_with_sniff(html);
        assert!(r.is_empty());
    }

    #[test]
    fn cloudflare_challenge_returns_empty() {
        let challenge =
            r#"<html><head><title>Just a moment...</title></head><body>cf-mitigated</body></html>"#;
        let r = parse_sitemap_with_sniff(challenge);
        assert!(r.is_empty());
    }

    #[test]
    fn sniff_handles_utf8_split_at_boundary() {
        // Pad with ASCII so byte 2048 lands inside a multi-byte UTF-8 char.
        // Each `é` is 2 bytes; positioning one to span 2047..2049 forces the
        // sniff window to land mid-character. Must NOT panic.
        let mut s = String::new();
        s.push_str(&"a".repeat(2047));
        s.push('é');
        s.push_str("<urlset><url><loc>https://x.com/p</loc></url></urlset>");
        // Should not panic, and should not be flagged as HTML.
        let r = parse_sitemap_with_sniff(&s);
        // The body has a urlset deeper than the sniff window, so the parser
        // still picks up the URL once we get past sniffing.
        assert!(r.page_urls.iter().any(|u| u.contains("/p")));
    }

    #[test]
    fn empty_xml_returns_empty_result() {
        let xml = "<?xml version=\"1.0\"?><urlset></urlset>";
        let r = parse_sitemap(xml);
        assert!(r.is_empty());
    }

    fn key(u: &str) -> OriginKey {
        origin_key(&url::Url::parse(u).unwrap()).unwrap()
    }

    #[test]
    fn origin_key_normalizes_case_and_default_port() {
        let a = key("https://example.com/x");
        let b = key("https://example.com:443/y");
        assert_eq!(a, b, "default port (443) is canonicalized");
        let c = key("https://Example.COM/");
        assert_eq!(a, c, "host is lowercased");
    }

    #[test]
    fn same_origin_url_treats_www_as_distinct() {
        // apex and www are different hosts at the URL-spec level. We do NOT
        // collapse them — a redirect/child-sitemap that crosses between them
        // is rejected by design.
        let apex = key("https://example.com/");
        assert!(same_origin_url("https://example.com/x", &apex));
        assert!(!same_origin_url("https://www.example.com/x", &apex));
        assert!(!same_origin_url("https://evil.com/x", &apex));
        assert!(!same_origin_url("https://cdn.example.com/x", &apex));

        let www = key("https://www.example.com/");
        assert!(same_origin_url("https://www.example.com/x", &www));
        assert!(!same_origin_url("https://example.com/x", &www));
    }

    #[test]
    fn same_origin_url_distinguishes_scheme_and_port() {
        let https = key("https://example.com/");
        // Different scheme: http vs https → must NOT match.
        assert!(!same_origin_url("http://example.com/x", &https));
        // Different port: explicit :8443 vs default :443 → must NOT match.
        assert!(!same_origin_url("https://example.com:8443/x", &https));
    }

    #[test]
    fn same_origin_url_blocks_non_http_schemes() {
        let target = key("https://example.com/");
        assert!(!same_origin_url("ftp://example.com/x", &target));
        assert!(!same_origin_url("file:///etc/passwd", &target));
    }

    #[test]
    fn decode_gzip_round_trips() {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use std::io::Write;
        let original =
            b"<?xml version=\"1.0\"?><urlset><url><loc>https://x.com/a</loc></url></urlset>";
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(original).unwrap();
        let gz = enc.finish().unwrap();
        let decoded = decode_gzip_capped(&gz, MAX_SITEMAP_BYTES).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn decode_gzip_rejects_bomb() {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use std::io::Write;
        // 2 MB of zeros compresses to a few KB, then expands back to 2 MB.
        let big = vec![0u8; 2 * 1024 * 1024];
        let mut enc = GzEncoder::new(Vec::new(), Compression::best());
        enc.write_all(&big).unwrap();
        let gz = enc.finish().unwrap();
        // Cap below decoded size → must error, not OOM.
        let err = decode_gzip_capped(&gz, 1024 * 1024).unwrap_err();
        assert!(err.contains("exceeds"));
    }

    #[tokio::test]
    async fn fetch_sitemap_handles_404() {
        let mock = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(404))
            .mount(&mock)
            .await;
        let client = reqwest::Client::new();
        let r = fetch_sitemap(&format!("{}/sitemap.xml", mock.uri()), &client)
            .await
            .unwrap();
        assert!(r.is_empty());
    }

    #[tokio::test]
    async fn fetch_sitemap_recursion_via_tree() {
        let mock = wiremock::MockServer::start().await;
        let host = mock.uri().replace("http://", "");

        // Index points at two children.
        let index_body = format!(
            r#"<?xml version="1.0"?><sitemapindex>
  <sitemap><loc>http://{host}/sm-1.xml</loc></sitemap>
  <sitemap><loc>http://{host}/sm-2.xml</loc></sitemap>
</sitemapindex>"#
        );
        let leaf_1 = format!(
            r#"<?xml version="1.0"?><urlset>
  <url><loc>http://{host}/page-a</loc></url>
  <url><loc>http://{host}/page-b</loc></url>
</urlset>"#
        );
        let leaf_2 = format!(
            r#"<?xml version="1.0"?><urlset>
  <url><loc>http://{host}/page-c</loc></url>
</urlset>"#
        );

        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/sitemap.xml"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(index_body))
            .mount(&mock)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/sm-1.xml"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(leaf_1))
            .mount(&mock)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/sm-2.xml"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(leaf_2))
            .mount(&mock)
            .await;

        let target = url::Url::parse(&mock.uri()).unwrap();
        let client = reqwest::Client::new();
        let urls = fetch_sitemap_tree(
            vec![format!("{}/sitemap.xml", mock.uri())],
            &target,
            &client,
            3,
            25,
            5000,
            8,
        )
        .await;

        let mut sorted = urls.clone();
        sorted.sort();
        assert_eq!(sorted.len(), 3, "expected 3 leaf page URLs, got {sorted:?}");
        assert!(sorted[0].ends_with("/page-a"));
        assert!(sorted[1].ends_with("/page-b"));
        assert!(sorted[2].ends_with("/page-c"));
    }

    #[tokio::test]
    async fn fetch_sitemap_tree_filters_cross_origin_children() {
        let mock = wiremock::MockServer::start().await;
        let host = mock.uri().replace("http://", "");

        // Index points at one same-host child + one cross-origin child.
        let index_body = format!(
            r#"<?xml version="1.0"?><sitemapindex>
  <sitemap><loc>http://{host}/sm-good.xml</loc></sitemap>
  <sitemap><loc>https://evil.com/sitemap.xml</loc></sitemap>
</sitemapindex>"#
        );
        let leaf_good = format!(
            r#"<?xml version="1.0"?><urlset>
  <url><loc>http://{host}/ok</loc></url>
</urlset>"#
        );

        wiremock::Mock::given(wiremock::matchers::path("/index.xml"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(index_body))
            .mount(&mock)
            .await;
        wiremock::Mock::given(wiremock::matchers::path("/sm-good.xml"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(leaf_good))
            .mount(&mock)
            .await;

        let target = url::Url::parse(&mock.uri()).unwrap();
        let client = reqwest::Client::new();
        let urls = fetch_sitemap_tree(
            vec![format!("{}/index.xml", mock.uri())],
            &target,
            &client,
            3,
            25,
            5000,
            8,
        )
        .await;

        // Cross-origin sitemap was filtered → only same-host pages returned.
        assert_eq!(urls.len(), 1);
        assert!(urls[0].ends_with("/ok"));
    }

    #[tokio::test]
    async fn fetch_sitemap_tree_breaks_cycle() {
        let mock = wiremock::MockServer::start().await;
        let host = mock.uri().replace("http://", "");

        // a -> b -> a cycle; visited set must terminate it.
        let a = format!(
            r#"<?xml version="1.0"?><sitemapindex>
  <sitemap><loc>http://{host}/b.xml</loc></sitemap>
</sitemapindex>"#
        );
        let b = format!(
            r#"<?xml version="1.0"?><sitemapindex>
  <sitemap><loc>http://{host}/a.xml</loc></sitemap>
</sitemapindex>"#
        );

        wiremock::Mock::given(wiremock::matchers::path("/a.xml"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(a))
            .mount(&mock)
            .await;
        wiremock::Mock::given(wiremock::matchers::path("/b.xml"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(b))
            .mount(&mock)
            .await;

        let target = url::Url::parse(&mock.uri()).unwrap();
        let client = reqwest::Client::new();
        let urls = fetch_sitemap_tree(
            vec![format!("{}/a.xml", mock.uri())],
            &target,
            &client,
            5,
            25,
            5000,
            8,
        )
        .await;
        assert!(urls.is_empty());
    }

    #[tokio::test]
    async fn fetch_sitemap_rejects_cross_origin_redirect() {
        // A same-origin seed (mock_a) 302s to a different mock host (mock_b).
        // Even though the redirect target is SSRF-safe, fetch_sitemap must
        // refuse to parse the body — otherwise an attacker could host a
        // sitemap on a different origin and have us follow into it.
        let mock_a = wiremock::MockServer::start().await;
        let mock_b = wiremock::MockServer::start().await;

        let body = r#"<?xml version="1.0"?><urlset>
  <url><loc>https://evil.com/owned</loc></url>
</urlset>"#;

        wiremock::Mock::given(wiremock::matchers::path("/sitemap.xml"))
            .respond_with(
                wiremock::ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/sitemap.xml", mock_b.uri())),
            )
            .mount(&mock_a)
            .await;
        wiremock::Mock::given(wiremock::matchers::path("/sitemap.xml"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(body))
            .mount(&mock_b)
            .await;

        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .unwrap();
        let result = fetch_sitemap(&format!("{}/sitemap.xml", mock_a.uri()), &client)
            .await
            .unwrap();
        assert!(
            result.is_empty(),
            "cross-origin redirect must yield empty result, got {result:?}"
        );
    }
}
