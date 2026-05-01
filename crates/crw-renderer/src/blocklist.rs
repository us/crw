//! Resource blocklist for CDP `Fetch.requestPaused` interception.
//!
//! Two axes: resource type (image / media / font / manifest / websocket /
//! stylesheet) and host substring (analytics / ads / session-replay vendors).
//! `should_block` is the pure decision used by the interception pump; everything
//! is data so it's straightforward to unit-test.

/// Resource types blocked by default. `stylesheet` is intentionally NOT in v1 —
/// it's behind a separate config flag because dropping CSS routinely breaks
/// truth_recall on JS-driven sites that gate hydration on stylesheet load.
pub const DEFAULT_BLOCKED_RESOURCE_TYPES: &[&str] =
    &["Image", "Media", "Font", "Manifest", "WebSocket"];

/// Host substrings (case-insensitive). Match if the request URL's host contains
/// any entry. Covers the analytics / ads / session-replay vendors that account
/// for the bulk of third-party request volume on most pages.
pub const DEFAULT_BLOCKED_HOSTS: &[&str] = &[
    "google-analytics.com",
    "googletagmanager.com",
    "doubleclick.net",
    "googleadservices.com",
    "googlesyndication.com",
    "hotjar.com",
    "segment.io",
    "segment.com",
    "amplitude.com",
    "mixpanel.com",
    "clarity.ms",
    "onetrust.com",
    "cookielaw.org",
    "criteo.com",
    "criteo.net",
    "taboola.com",
    "outbrain.com",
    "adsystem.com",
    "adservice.google.com",
    "scorecardresearch.com",
    "quantserve.com",
    "chartbeat.com",
    "nr-data.net",
    "newrelic.com",
];

/// Outcome of a blocklist check. Mirrors the metric label set so the pump can
/// emit `chrome_blocked_requests_total{reason}` directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    ResourceType,
    Host,
}

#[derive(Debug, Clone)]
pub struct Blocklist {
    resource_types: Vec<String>,
    host_substrings: Vec<String>,
    block_stylesheets: bool,
}

impl Blocklist {
    /// Default blocklist: resource types + host substrings, no stylesheets.
    pub fn defaults() -> Self {
        Self {
            resource_types: DEFAULT_BLOCKED_RESOURCE_TYPES
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            host_substrings: DEFAULT_BLOCKED_HOSTS
                .iter()
                .map(|s| s.to_lowercase())
                .collect(),
            block_stylesheets: false,
        }
    }

    /// Toggle stylesheet blocking. Off by default — see module docs.
    pub fn with_stylesheets(mut self, on: bool) -> Self {
        self.block_stylesheets = on;
        self
    }

    /// Empty blocklist — every request gets `continueRequest`. Used when the
    /// host is on the per-host opt-out list.
    pub fn empty() -> Self {
        Self {
            resource_types: Vec::new(),
            host_substrings: Vec::new(),
            block_stylesheets: false,
        }
    }

    /// Decide whether a request should be blocked. `resource_type` and `url`
    /// are the values reported by `Fetch.requestPaused`.
    pub fn should_block(&self, resource_type: &str, url: &str) -> Option<BlockReason> {
        if self.resource_types.iter().any(|t| t == resource_type) {
            return Some(BlockReason::ResourceType);
        }
        if self.block_stylesheets && resource_type == "Stylesheet" {
            return Some(BlockReason::ResourceType);
        }
        if !self.host_substrings.is_empty()
            && let Ok(parsed) = url::Url::parse(url)
            && let Some(host) = parsed.host_str()
        {
            let host_lc = host.to_lowercase();
            if self.host_substrings.iter().any(|h| host_lc.contains(h)) {
                return Some(BlockReason::Host);
            }
        }
        None
    }
}

impl Default for Blocklist {
    fn default() -> Self {
        Self::defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_image_resource_type() {
        let bl = Blocklist::defaults();
        assert_eq!(
            bl.should_block("Image", "https://example.com/cat.jpg"),
            Some(BlockReason::ResourceType),
        );
    }

    #[test]
    fn blocks_font_resource_type() {
        let bl = Blocklist::defaults();
        assert_eq!(
            bl.should_block("Font", "https://example.com/x.woff2"),
            Some(BlockReason::ResourceType),
        );
    }

    #[test]
    fn allows_document_resource_type() {
        let bl = Blocklist::defaults();
        assert_eq!(bl.should_block("Document", "https://example.com/"), None);
    }

    #[test]
    fn allows_stylesheet_by_default() {
        let bl = Blocklist::defaults();
        assert_eq!(
            bl.should_block("Stylesheet", "https://example.com/x.css"),
            None
        );
    }

    #[test]
    fn blocks_stylesheet_when_enabled() {
        let bl = Blocklist::defaults().with_stylesheets(true);
        assert_eq!(
            bl.should_block("Stylesheet", "https://example.com/x.css"),
            Some(BlockReason::ResourceType),
        );
    }

    #[test]
    fn blocks_google_analytics_host() {
        let bl = Blocklist::defaults();
        assert_eq!(
            bl.should_block("XHR", "https://www.google-analytics.com/g/collect"),
            Some(BlockReason::Host),
        );
    }

    #[test]
    fn blocks_doubleclick_host_case_insensitive() {
        let bl = Blocklist::defaults();
        assert_eq!(
            bl.should_block("Script", "https://AD.DOUBLECLICK.NET/track"),
            Some(BlockReason::Host),
        );
    }

    #[test]
    fn empty_blocklist_allows_everything() {
        let bl = Blocklist::empty();
        assert_eq!(bl.should_block("Image", "https://example.com/x.jpg"), None);
        assert_eq!(
            bl.should_block("Script", "https://google-analytics.com/g"),
            None,
        );
    }

    #[test]
    fn malformed_url_does_not_panic() {
        let bl = Blocklist::defaults();
        // Resource-type check still fires.
        assert_eq!(
            bl.should_block("Image", "not a url"),
            Some(BlockReason::ResourceType)
        );
        // Host check no-ops on parse failure.
        assert_eq!(bl.should_block("Script", "not a url"), None);
    }
}
