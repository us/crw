//! Compile-time default deny-/preserve-lists for the /map URL filter.
//!
//! See `url_filter.rs` for how these are consulted. Lists below are sourced
//! from ClearURLs `globalRules` + research-agent taxonomy (WooCommerce/WP
//! action params, common analytics trackers). All keys are ASCII lowercase.

use phf::{Set, phf_set};

/// Action params: presence of any matching KEY in the query causes the URL
/// to be dropped entirely (Tier A).
pub static DEFAULT_ACTION_PARAMS: Set<&'static str> = phf_set! {
    // WooCommerce — Issue #40's primary trigger
    "add-to-cart", "remove_item", "undo_item", "removed_item",
    "wc-ajax", "wc-api", "pay_for_order", "apply_coupon",
    "remove_coupon", "order_again", "delete_payment_method",
    // Wishlist plugins (YITH, TI)
    "add_to_wishlist", "remove_from_wishlist", "yith_wcwl_action",
    // WordPress core
    "_wpnonce", "_wp_http_referer", "replytocom",
    "unapproved", "moderation-hash", "doing_wp_cron",
    "customize_changeset_uuid", "customize_messenger_channel",
    "preview_id", "preview_nonce",
    // Magento 2
    "isajax", "form_key",
    // Anti-CSRF tokens — universal; no CMS uses these as content keys
    "nonce", "csrf", "csrf_token", "authenticity_token",
};

/// Tracking params: stripped from the query (Tier B); URL is kept.
pub static DEFAULT_TRACKING_PARAMS: Set<&'static str> = phf_set! {
    // Google
    "utm_source", "utm_medium", "utm_campaign", "utm_term", "utm_content",
    "utm_id", "utm_name", "utm_brand", "utm_social", "utm_social-type",
    "gclid", "gclsrc", "gbraid", "wbraid", "dclid", "gad_source", "srsltid",
    "_ga", "_gl",
    // Facebook / Meta
    "fbclid",
    // Microsoft / Bing
    "msclkid",
    // TikTok
    "ttclid",
    // LinkedIn
    "li_fat_id",
    // Twitter / X
    "twclid", "__twitter_impression",
    // Yandex
    "yclid", "ysclid",
    // HubSpot
    "_hsenc", "_hsmi", "__hssc", "__hstc", "__hsfp", "hsctatracking",
    // Mailchimp
    "mc_cid", "mc_eid",
    // Matomo / Piwik
    "mtm_source", "mtm_medium", "mtm_campaign", "mtm_keyword", "mtm_cid",
    "pk_campaign", "pk_kwd", "pk_source", "pk_medium",
    // Marketo
    "mkt_tok",
    // Instagram
    "igshid",
    // Session IDs — strip, don't drop. Legacy session-via-URL sites need
    // the URL preserved without the session token.
    "sessionid", "phpsessid", "jsessionid",
};

/// Universal preserve set: Tier B always skips these. Tier A still runs.
/// Trimmed of generic keys (`id`, `t`, `v`, `tag`, `s`, `q`, ...) that are
/// too ambiguous globally — those move to host-scoped overrides instead.
pub static ALWAYS_PRESERVE: Set<&'static str> = phf_set! {
    "p", "page_id", "page", "paged", "offset",
    "category", "cat",
    "lang", "locale", "hl",
    "title", "docid", "caseid", "oldid",
};

/// Host pattern. Anchored matchers — no regex in v1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostPat {
    /// Match any host.
    Any,
    /// Exact host equality, lowercased.
    Exact(&'static str),
    /// Suffix match. Pattern includes the leading `.` for clarity
    /// (e.g. `.reddit.com` matches `old.reddit.com` and `www.reddit.com`).
    Suffix(&'static str),
}

impl HostPat {
    pub fn matches(&self, host: &str) -> bool {
        match self {
            HostPat::Any => true,
            HostPat::Exact(h) => host.eq_ignore_ascii_case(h),
            HostPat::Suffix(suf) => {
                host.len() > suf.len()
                    && host
                        .get(host.len() - suf.len()..)
                        .map(|tail| tail.eq_ignore_ascii_case(suf))
                        .unwrap_or(false)
            }
        }
    }
}

/// Compile-time host-override entry. Built into runtime `HostOverride` at
/// config-load time (string slices → owned `HashSet<String>` once).
#[derive(Debug, Clone)]
pub struct HostOverrideEntry {
    pub host_pat: HostPat,
    /// Path-substring guards. Empty = any path on the host.
    pub when_path_contains: &'static [&'static str],
    pub preserve_params: &'static [&'static str],
    pub exempt_action_params: &'static [&'static str],
    pub extra_tracking_params: &'static [&'static str],
}

pub static DEFAULT_HOST_OVERRIDES: &[HostOverrideEntry] = &[
    // phpBB / vBulletin / IPB — thread/post IDs are content
    HostOverrideEntry {
        host_pat: HostPat::Any,
        when_path_contains: &["viewtopic.php", "showthread.php", "showtopic"],
        preserve_params: &["t", "p", "f", "topic", "thread", "tid", "pid"],
        exempt_action_params: &[],
        extra_tracking_params: &[],
    },
    // MediaWiki — preserve title/oldid/diff/curid against tracker strip.
    HostOverrideEntry {
        host_pat: HostPat::Any,
        when_path_contains: &["/wiki/", "/w/index.php"],
        preserve_params: &["title", "oldid", "diff", "curid"],
        exempt_action_params: &[],
        extra_tracking_params: &[],
    },
    // YouTube watch pages
    HostOverrideEntry {
        host_pat: HostPat::Exact("youtube.com"),
        when_path_contains: &["/watch"],
        preserve_params: &["v", "list", "t"],
        exempt_action_params: &[],
        extra_tracking_params: &[],
    },
    HostOverrideEntry {
        host_pat: HostPat::Suffix(".youtube.com"),
        when_path_contains: &["/watch"],
        preserve_params: &["v", "list", "t"],
        exempt_action_params: &[],
        extra_tracking_params: &[],
    },
    // Reddit
    HostOverrideEntry {
        host_pat: HostPat::Suffix(".reddit.com"),
        when_path_contains: &["/comments/", "/r/"],
        preserve_params: &["context", "sort", "depth"],
        exempt_action_params: &[],
        extra_tracking_params: &[],
    },
    // Shopify storefront — only here do these strip; outside they pass through.
    HostOverrideEntry {
        host_pat: HostPat::Suffix(".myshopify.com"),
        when_path_contains: &[],
        preserve_params: &[],
        exempt_action_params: &[],
        extra_tracking_params: &["_pos", "_psq", "_ss", "_sid", "_v", "_fid", "_fd"],
    },
];

/// Host suffixes that, by default, suppress Tier A action-drops only.
/// Tier B (tracking strip) still runs. Toggle off via `gov_tld_drop_actions`.
pub static GOV_TLD_SUFFIXES: &[&str] = &[".gov", ".gov.uk", ".mil", "europa.eu"];

#[cfg(test)]
mod audit {
    use super::*;

    /// Action and tracking lists must be disjoint — otherwise behavior is
    /// ambiguous (Tier A drops the URL before Tier B runs, but a key being
    /// in both means there's editorial confusion in the lists).
    #[test]
    fn action_and_tracking_disjoint() {
        for k in DEFAULT_ACTION_PARAMS.iter() {
            assert!(
                !DEFAULT_TRACKING_PARAMS.contains(*k),
                "key {:?} present in both ACTION and TRACKING lists",
                k
            );
        }
    }

    #[test]
    fn preserve_does_not_overlap_action() {
        for k in ALWAYS_PRESERVE.iter() {
            assert!(
                !DEFAULT_ACTION_PARAMS.contains(*k),
                "key {:?} present in both ALWAYS_PRESERVE and ACTION lists",
                k
            );
        }
    }

    #[test]
    fn all_keys_are_lowercase() {
        for k in DEFAULT_ACTION_PARAMS.iter() {
            assert_eq!(
                *k,
                k.to_ascii_lowercase(),
                "action key {:?} not lowercase",
                k
            );
        }
        for k in DEFAULT_TRACKING_PARAMS.iter() {
            assert_eq!(
                *k,
                k.to_ascii_lowercase(),
                "tracking key {:?} not lowercase",
                k
            );
        }
        for k in ALWAYS_PRESERVE.iter() {
            assert_eq!(
                *k,
                k.to_ascii_lowercase(),
                "preserve key {:?} not lowercase",
                k
            );
        }
    }

    #[test]
    fn host_suffix_patterns_start_with_dot() {
        for entry in DEFAULT_HOST_OVERRIDES {
            if let HostPat::Suffix(s) = entry.host_pat {
                assert!(
                    s.starts_with('.'),
                    "suffix pattern {:?} should start with `.`",
                    s
                );
            }
        }
    }

    #[test]
    fn host_pat_suffix_match_does_not_match_substring() {
        // ".com" should not match "example.com.attacker.tld"
        let p = HostPat::Suffix(".reddit.com");
        assert!(p.matches("old.reddit.com"));
        assert!(p.matches("www.reddit.com"));
        assert!(!p.matches("reddit.com")); // suffix requires content before
        assert!(!p.matches("notreddit.com"));
        assert!(!p.matches("reddit.com.attacker.tld"));
    }

    #[test]
    fn host_pat_exact_is_case_insensitive() {
        let p = HostPat::Exact("youtube.com");
        assert!(p.matches("youtube.com"));
        assert!(p.matches("YouTube.com"));
        assert!(!p.matches("www.youtube.com"));
    }
}
