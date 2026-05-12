//! /map URL filter pipeline.
//!
//! Tier A — drop URL outright if any query key matches the action deny-list.
//! Tier B — strip tracking-param keys from the query, keep the URL.
//! Tier C — host-scoped overrides preserve/exempt/inject extras.
//!
//! Entry points:
//! - [`filter_and_normalize_raw`]: sitemap path (no pre-parsed Url available).
//! - [`filter_and_normalize_parsed`]: BFS path (reuses caller's `url::Url`).
//!
//! Both return `None` if Tier A drops the URL.

use crate::url_filter_data::{
    ALWAYS_PRESERVE, DEFAULT_ACTION_PARAMS, DEFAULT_HOST_OVERRIDES, DEFAULT_TRACKING_PARAMS,
    GOV_TLD_SUFFIXES, HostOverrideEntry, HostPat,
};
use crw_core::metrics::metrics;
use std::collections::HashSet;

/// Per-request override delta. `None` fields fall through to whatever the
/// server's default cfg already specifies. Construction lives in the route
/// handler — it owns the precedence resolution (request > TOML > default).
#[derive(Debug, Clone, Default)]
pub struct RequestOverrides {
    pub strip_tracking: Option<bool>,
    pub drop_actions: Option<bool>,
    /// Firecrawl-compatible coarse alias. Outermost gate: `Some(_)` makes
    /// `strip_tracking` / `drop_actions` ignored.
    pub coarse_strip_all: Option<bool>,
    pub extra_tracking: Option<Vec<String>>,
    pub extra_action: Option<Vec<String>>,
    pub preserve: Option<Vec<String>>,
}

/// Runtime host-override entry (owned strings) built once at config-load time.
#[derive(Debug, Clone)]
pub struct HostOverride {
    pub host_pat: HostPat,
    pub when_path_contains: Vec<String>,
    pub preserve_params: HashSet<String>,
    pub exempt_action_params: HashSet<String>,
    pub extra_tracking_params: HashSet<String>,
}

impl HostOverride {
    fn from_static(e: &HostOverrideEntry) -> Self {
        Self {
            host_pat: e.host_pat.clone(),
            when_path_contains: e.when_path_contains.iter().map(|s| s.to_string()).collect(),
            preserve_params: e.preserve_params.iter().map(|s| s.to_string()).collect(),
            exempt_action_params: e
                .exempt_action_params
                .iter()
                .map(|s| s.to_string())
                .collect(),
            extra_tracking_params: e
                .extra_tracking_params
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

/// Filter configuration. Built once at server startup, shared via `Arc`.
#[derive(Debug, Clone)]
pub struct UrlFilterCfg {
    pub strip_tracking: bool,
    pub drop_actions: bool,
    /// Firecrawl-compatible coarse mode: strip every non-preserved param.
    pub coarse_strip_all: bool,
    /// When true, `.gov`/`.mil` etc. hosts run Tier A too. Default false:
    /// gov sites preserve action URLs (govspeak forms etc.).
    pub gov_tld_drop_actions: bool,
    /// User-supplied extras, additive on top of `DEFAULT_TRACKING_PARAMS`.
    /// Pre-lowercased at build time.
    pub tracking_params: HashSet<String>,
    pub action_params: HashSet<String>,
    pub preserve_params: HashSet<String>,
    pub host_overrides: Vec<HostOverride>,
}

impl Default for UrlFilterCfg {
    fn default() -> Self {
        Self::defaults_on()
    }
}

impl UrlFilterCfg {
    /// Plan default: Tier A + Tier B both active, gov suppression on,
    /// coarse mode off, compiled-in host overrides loaded.
    pub fn defaults_on() -> Self {
        Self {
            strip_tracking: true,
            drop_actions: true,
            coarse_strip_all: false,
            gov_tld_drop_actions: false,
            tracking_params: HashSet::new(),
            action_params: HashSet::new(),
            preserve_params: HashSet::new(),
            host_overrides: DEFAULT_HOST_OVERRIDES
                .iter()
                .map(HostOverride::from_static)
                .collect(),
        }
    }

    /// Build from the raw TOML `[map.url_filter]` block. Strings are
    /// lowercased once here so per-request lookups stay allocation-free.
    pub fn from_map_config(cfg: &crw_core::config::MapUrlFilterConfig) -> Self {
        let to_lower_set = |v: &[String]| -> HashSet<String> {
            v.iter().map(|s| s.to_ascii_lowercase()).collect()
        };
        Self {
            strip_tracking: cfg.strip_tracking_params,
            drop_actions: cfg.drop_action_urls,
            coarse_strip_all: false,
            gov_tld_drop_actions: cfg.gov_tld_drop_actions,
            tracking_params: to_lower_set(&cfg.extra_tracking_params),
            action_params: to_lower_set(&cfg.extra_action_params),
            preserve_params: to_lower_set(&cfg.extra_preserve_params),
            host_overrides: DEFAULT_HOST_OVERRIDES
                .iter()
                .map(HostOverride::from_static)
                .collect(),
        }
    }

    /// Apply request-level overrides on top of `self`. Returns a fresh
    /// `UrlFilterCfg`; the input is not mutated so the server's Arc'd
    /// default stays shareable across concurrent requests.
    pub fn with_overrides(&self, ov: RequestOverrides) -> Self {
        let mut out = self.clone();
        if let Some(coarse) = ov.coarse_strip_all {
            out.coarse_strip_all = coarse;
            if !coarse {
                // Coarse `false` is the explicit "give me raw URLs" escape
                // hatch — switch both tiers off.
                out.strip_tracking = false;
                out.drop_actions = false;
                return out;
            }
            // Coarse `true` overrides granular flags (Tier A still runs).
            return out;
        }
        if let Some(v) = ov.strip_tracking {
            out.strip_tracking = v;
        }
        if let Some(v) = ov.drop_actions {
            out.drop_actions = v;
        }
        if let Some(extra) = ov.extra_tracking {
            for k in extra {
                out.tracking_params.insert(k.to_ascii_lowercase());
            }
        }
        if let Some(extra) = ov.extra_action {
            for k in extra {
                out.action_params.insert(k.to_ascii_lowercase());
            }
        }
        if let Some(extra) = ov.preserve {
            for k in extra {
                out.preserve_params.insert(k.to_ascii_lowercase());
            }
        }
        out
    }

    /// Both tiers off — pass-through (other than `normalize_url`).
    pub fn off() -> Self {
        Self {
            strip_tracking: false,
            drop_actions: false,
            coarse_strip_all: false,
            gov_tld_drop_actions: false,
            tracking_params: HashSet::new(),
            action_params: HashSet::new(),
            preserve_params: HashSet::new(),
            host_overrides: Vec::new(),
        }
    }
}

/// Mirror of `crawl::normalize_url` — kept private to this module so the
/// filter can be exercised in unit tests without crossing module boundaries.
fn normalize(url: &str) -> String {
    let without_fragment = url.split('#').next().unwrap_or(url);
    without_fragment.trim_end_matches('/').to_lowercase()
}

/// Returns `host` is matched by any compiled `.gov`/`.mil` suffix.
fn is_gov_host(host: &str) -> bool {
    GOV_TLD_SUFFIXES.iter().any(|suf| {
        if let Some(stripped) = suf.strip_prefix('.') {
            host.eq_ignore_ascii_case(stripped)
                || host.len() > suf.len()
                    && host
                        .get(host.len() - suf.len()..)
                        .map(|t| t.eq_ignore_ascii_case(suf))
                        .unwrap_or(false)
        } else {
            host.eq_ignore_ascii_case(suf)
                || host.len() > suf.len() + 1
                    && host
                        .get(host.len() - suf.len() - 1..)
                        .map(|t| t.eq_ignore_ascii_case(&format!(".{}", suf)))
                        .unwrap_or(false)
        }
    })
}

/// Sitemap entry point — no pre-parsed URL. Cheap pre-screen avoids
/// the `Url::parse` cost for the overwhelming majority of internal links
/// that have no `?`.
pub fn filter_and_normalize_raw(url: &str, cfg: &UrlFilterCfg) -> Option<String> {
    if cfg.coarse_strip_all || cfg.strip_tracking || cfg.drop_actions {
        if !url.contains('?') {
            return Some(normalize(url));
        }
        let parsed = match url::Url::parse(url) {
            Ok(u) => u,
            Err(_) => {
                metrics()
                    .map_filter_dropped_total
                    .with_label_values(&["parse_error_passthrough"])
                    .inc();
                return Some(normalize(url));
            }
        };
        filter_and_normalize_parsed(&parsed, url, cfg)
    } else {
        Some(normalize(url))
    }
}

/// BFS entry point — caller already has a parsed `Url`.
pub fn filter_and_normalize_parsed(
    parsed: &url::Url,
    raw: &str,
    cfg: &UrlFilterCfg,
) -> Option<String> {
    // No query — nothing for either tier to do.
    if parsed.query().is_none() {
        return Some(normalize(raw));
    }
    // Both tiers off and no coarse — pass through.
    if !cfg.coarse_strip_all && !cfg.strip_tracking && !cfg.drop_actions {
        return Some(normalize(raw));
    }

    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
    let path = parsed.path();

    // Resolve effective sets from host overrides.
    let mut eff_preserve: HashSet<String> = cfg.preserve_params.clone();
    let mut eff_exempt_action: HashSet<String> = HashSet::new();
    let mut eff_extra_tracking: HashSet<String> = HashSet::new();
    let mut host_override_hit = false;
    for ov in &cfg.host_overrides {
        if !ov.host_pat.matches(&host) {
            continue;
        }
        let path_ok = ov.when_path_contains.is_empty()
            || ov
                .when_path_contains
                .iter()
                .any(|s| path.contains(s.as_str()));
        if !path_ok {
            continue;
        }
        host_override_hit = true;
        eff_preserve.extend(ov.preserve_params.iter().cloned());
        eff_exempt_action.extend(ov.exempt_action_params.iter().cloned());
        eff_extra_tracking.extend(ov.extra_tracking_params.iter().cloned());
    }

    let gov = is_gov_host(&host);

    // Iterate raw query: preserves original percent-encoding and
    // distinguishes `?k` (no `=`) from `?k=`.
    let raw_query = parsed.query().unwrap_or("");
    let pairs: Vec<(String, &str, bool)> = raw_query
        .split('&')
        .filter(|s| !s.is_empty())
        .map(|p| match p.find('=') {
            Some(i) => (p[..i].to_ascii_lowercase(), p, true),
            None => (p.to_ascii_lowercase(), p, false),
        })
        .collect();

    // Tier A: action-URL drop.
    let drop_actions_active = cfg.drop_actions && (cfg.gov_tld_drop_actions || !gov);
    if drop_actions_active {
        for (kl, _raw_pair, _) in &pairs {
            if eff_preserve.contains(kl) || ALWAYS_PRESERVE.contains(kl.as_str()) {
                continue;
            }
            if eff_exempt_action.contains(kl) {
                continue;
            }
            let in_action =
                cfg.action_params.contains(kl) || DEFAULT_ACTION_PARAMS.contains(kl.as_str());
            if in_action {
                metrics()
                    .map_filter_dropped_total
                    .with_label_values(&["action_param"])
                    .inc();
                return None;
            }
        }
    } else if gov && cfg.drop_actions {
        // Tier A suppressed by .gov rule — bookkeep only if an action key was
        // actually present; bounded label cardinality.
        let saw_action = pairs.iter().any(|(kl, _, _)| {
            cfg.action_params.contains(kl) || DEFAULT_ACTION_PARAMS.contains(kl.as_str())
        });
        if saw_action {
            metrics()
                .map_filter_preserved_total
                .with_label_values(&["gov_tld"])
                .inc();
        }
    }

    // Coarse mode — strip everything except preserves.
    if cfg.coarse_strip_all {
        let kept: Vec<&str> = pairs
            .iter()
            .filter(|(kl, _, _)| eff_preserve.contains(kl) || ALWAYS_PRESERVE.contains(kl.as_str()))
            .map(|(_, raw, _)| *raw)
            .collect();
        let stripped_any = kept.len() != pairs.len();
        if stripped_any {
            metrics()
                .map_filter_stripped_total
                .with_label_values(&["coarse_ignore"])
                .inc();
        }
        return Some(rebuild(parsed, &kept));
    }

    // Tier B: tracking strip.
    if cfg.strip_tracking {
        let mut kept: Vec<&str> = Vec::with_capacity(pairs.len());
        let mut stripped_any = false;
        for (kl, raw_pair, _) in &pairs {
            let always_pres = ALWAYS_PRESERVE.contains(kl.as_str());
            let host_pres = eff_preserve.contains(kl);
            if always_pres || host_pres {
                if host_pres && host_override_hit {
                    metrics()
                        .map_filter_preserved_total
                        .with_label_values(&["host_override"])
                        .inc();
                } else if always_pres {
                    metrics()
                        .map_filter_preserved_total
                        .with_label_values(&["always_preserve"])
                        .inc();
                }
                kept.push(raw_pair);
                continue;
            }
            let is_tracking = cfg.tracking_params.contains(kl)
                || DEFAULT_TRACKING_PARAMS.contains(kl.as_str())
                || eff_extra_tracking.contains(kl);
            if is_tracking {
                stripped_any = true;
                continue;
            }
            kept.push(raw_pair);
        }
        if stripped_any {
            metrics()
                .map_filter_stripped_total
                .with_label_values(&["tracking_param"])
                .inc();
        }
        return Some(rebuild(parsed, &kept));
    }

    // Tier A only, no strip configured — return URL with original query intact.
    Some(normalize(raw))
}

/// Re-emit URL with the surviving params in original order. Preserves raw
/// percent-encoding; drops the `?` entirely when empty.
fn rebuild(parsed: &url::Url, kept: &[&str]) -> String {
    let mut out = parsed.clone();
    if kept.is_empty() {
        out.set_query(None);
    } else {
        out.set_query(Some(&kept.join("&")));
    }
    out.set_fragment(None);
    normalize(out.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_on() -> UrlFilterCfg {
        UrlFilterCfg::defaults_on()
    }

    #[test]
    fn no_query_fast_path() {
        let cfg = cfg_on();
        let out = filter_and_normalize_raw("https://example.com/foo", &cfg).unwrap();
        assert_eq!(out, "https://example.com/foo");
    }

    #[test]
    fn action_param_drops_url() {
        let cfg = cfg_on();
        assert!(
            filter_and_normalize_raw("https://shop.example.com/?add-to-cart=360", &cfg).is_none()
        );
    }

    #[test]
    fn wpnonce_drops_url() {
        let cfg = cfg_on();
        assert!(
            filter_and_normalize_raw(
                "https://shop.example.com/?add_to_wishlist=6241&_wpnonce=b7643da9b9",
                &cfg
            )
            .is_none()
        );
    }

    #[test]
    fn case_insensitive_action_key() {
        let cfg = cfg_on();
        for u in [
            "https://e.test/?ADD-TO-CART=1",
            "https://e.test/?Add-To-Cart=1",
            "https://e.test/?add-to-cart=1",
        ] {
            assert!(
                filter_and_normalize_raw(u, &cfg).is_none(),
                "expected drop for {u}"
            );
        }
    }

    #[test]
    fn tracking_param_stripped_url_kept() {
        let cfg = cfg_on();
        let out =
            filter_and_normalize_raw("https://example.com/blog?utm_source=fb&fbclid=abc", &cfg)
                .unwrap();
        assert_eq!(out, "https://example.com/blog");
    }

    #[test]
    fn always_preserve_keys_survive() {
        let cfg = cfg_on();
        let out =
            filter_and_normalize_raw("https://wp.example.com/?p=123&utm_source=x", &cfg).unwrap();
        assert!(out.contains("p=123"), "got {out}");
        assert!(!out.contains("utm_source"), "got {out}");
    }

    #[test]
    fn action_wins_when_coexists_with_tracking() {
        let cfg = cfg_on();
        let out = filter_and_normalize_raw("https://e.test/?add-to-cart=1&utm_source=x", &cfg);
        assert!(out.is_none());
    }

    #[test]
    fn empty_query_after_strip_drops_question_mark() {
        let cfg = cfg_on();
        let out = filter_and_normalize_raw("https://example.com/blog?utm_source=fb", &cfg).unwrap();
        assert_eq!(out, "https://example.com/blog");
    }

    #[test]
    fn empty_value_action_key() {
        // ?_wpnonce — no equals sign — should still drop.
        let cfg = cfg_on();
        assert!(filter_and_normalize_raw("https://e.test/?_wpnonce", &cfg).is_none());
    }

    #[test]
    fn repeated_tracking_keys_all_stripped() {
        let cfg = cfg_on();
        let out =
            filter_and_normalize_raw("https://e.test/blog?utm_source=a&utm_source=b&p=1", &cfg)
                .unwrap();
        assert!(out.contains("p=1"));
        assert!(!out.contains("utm_source"));
    }

    #[test]
    fn malformed_url_passthrough() {
        let cfg = cfg_on();
        // Has `?` so it triggers the parse path; "not a url" fails parse.
        let out = filter_and_normalize_raw("not-a-url?utm=1", &cfg);
        assert!(out.is_some());
    }

    #[test]
    fn host_override_phpbb_preserves_thread_ids() {
        let cfg = cfg_on();
        let out = filter_and_normalize_raw(
            "https://forum.example.com/viewtopic.php?t=123&utm_source=x",
            &cfg,
        )
        .unwrap();
        assert!(out.contains("t=123"), "got {out}");
        assert!(!out.contains("utm_source"));
    }

    #[test]
    fn gov_tld_tier_a_suppressed_tier_b_runs() {
        let cfg = cfg_on();
        let out = filter_and_normalize_raw(
            "https://senate.gov/?docid=123&utm_source=x&add-to-cart=1",
            &cfg,
        )
        .unwrap();
        assert!(out.contains("docid=123"), "got {out}");
        assert!(out.contains("add-to-cart=1"), "got {out}");
        assert!(!out.contains("utm_source"), "got {out}");
    }

    #[test]
    fn gov_tld_opt_in_runs_tier_a() {
        let mut cfg = cfg_on();
        cfg.gov_tld_drop_actions = true;
        let out = filter_and_normalize_raw("https://senate.gov/?add-to-cart=1", &cfg);
        assert!(out.is_none());
    }

    #[test]
    fn shopify_host_strips_storefront_keys() {
        let cfg = cfg_on();
        let out = filter_and_normalize_raw(
            "https://shop.myshopify.com/products/x?_pos=1&_sid=abc&_v=1.0",
            &cfg,
        )
        .unwrap();
        assert!(!out.contains("_pos"));
        assert!(!out.contains("_sid"));
        assert!(!out.contains("_v="));
    }

    #[test]
    fn shopify_keys_pass_through_on_other_hosts() {
        let cfg = cfg_on();
        let out = filter_and_normalize_raw("https://random.com/?_pos=1", &cfg).unwrap();
        assert!(out.contains("_pos=1"));
    }

    #[test]
    fn youtube_watch_preserves_v_list_t() {
        let cfg = cfg_on();
        let out = filter_and_normalize_raw(
            "https://www.youtube.com/watch?v=abc123&list=PL1&utm_source=x",
            &cfg,
        )
        .unwrap();
        assert!(out.contains("v=abc123"), "got {out}");
        assert!(
            out.contains("list=pl1") || out.contains("list=PL1"),
            "got {out}"
        );
        assert!(!out.contains("utm_source"));
    }

    #[test]
    fn coarse_strip_all_drops_everything_except_preserve() {
        let mut cfg = cfg_on();
        cfg.coarse_strip_all = true;
        let out = filter_and_normalize_raw("https://e.test/blog?p=1&random=foo&utm_source=x", &cfg)
            .unwrap();
        assert!(out.contains("p=1"));
        assert!(!out.contains("random"));
        assert!(!out.contains("utm_source"));
    }

    #[test]
    fn coarse_mode_still_runs_tier_a() {
        let mut cfg = cfg_on();
        cfg.coarse_strip_all = true;
        let out = filter_and_normalize_raw("https://e.test/?add-to-cart=1", &cfg);
        assert!(out.is_none());
    }

    #[test]
    fn off_config_returns_raw_normalized() {
        let cfg = UrlFilterCfg::off();
        let out = filter_and_normalize_raw("https://e.test/blog?utm_source=x&add-to-cart=1", &cfg)
            .unwrap();
        assert!(out.contains("utm_source=x"));
        assert!(out.contains("add-to-cart=1"));
    }

    #[test]
    fn extra_action_param_drops_url() {
        let mut cfg = cfg_on();
        cfg.action_params.insert("custom_action".to_string());
        let out = filter_and_normalize_raw("https://e.test/?custom_action=1", &cfg);
        assert!(out.is_none());
    }

    #[test]
    fn extra_preserve_protects_against_default_action() {
        let mut cfg = cfg_on();
        cfg.preserve_params.insert("add-to-cart".to_string());
        let out = filter_and_normalize_raw("https://e.test/?add-to-cart=1", &cfg).unwrap();
        assert!(out.contains("add-to-cart=1"));
    }

    #[test]
    fn fragment_stripped() {
        let cfg = cfg_on();
        let out =
            filter_and_normalize_raw("https://e.test/blog?utm_source=x#section", &cfg).unwrap();
        assert!(!out.contains('#'));
    }

    // ─────────────────────────── property tests ──────────────────────────
    use proptest::prelude::*;

    /// Generate a query-string-safe ASCII key.
    fn arb_key() -> impl Strategy<Value = String> {
        "[a-zA-Z_][a-zA-Z0-9_-]{0,15}".prop_map(String::from)
    }

    fn arb_pair() -> impl Strategy<Value = (String, String)> {
        (arb_key(), "[a-zA-Z0-9._~-]{0,12}".prop_map(String::from))
    }

    proptest! {
        /// Output is either `None` (dropped) or a parseable URL.
        #[test]
        fn output_always_valid(pairs in prop::collection::vec(arb_pair(), 0..6)) {
            let cfg = cfg_on();
            let q: String = pairs
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("&");
            let url = if q.is_empty() {
                "https://example.com/path".to_string()
            } else {
                format!("https://example.com/path?{q}")
            };
            if let Some(out) = filter_and_normalize_raw(&url, &cfg) {
                prop_assert!(url::Url::parse(&out).is_ok(), "output not a valid URL: {out}");
            }
        }

        /// Idempotency: filter ∘ filter == filter.
        #[test]
        fn filter_idempotent(pairs in prop::collection::vec(arb_pair(), 0..6)) {
            let cfg = cfg_on();
            let q: String = pairs
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("&");
            let url = if q.is_empty() {
                "https://example.com/path".to_string()
            } else {
                format!("https://example.com/path?{q}")
            };
            let once = filter_and_normalize_raw(&url, &cfg);
            if let Some(o) = &once {
                let twice = filter_and_normalize_raw(o, &cfg);
                prop_assert_eq!(twice.as_ref(), Some(o));
            }
        }

        /// Length non-increasing: surviving query ≤ input query length.
        #[test]
        fn length_non_increasing(pairs in prop::collection::vec(arb_pair(), 0..6)) {
            let cfg = cfg_on();
            let q: String = pairs
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("&");
            if q.is_empty() {
                return Ok(());
            }
            let url = format!("https://example.com/path?{q}");
            if let Some(out) = filter_and_normalize_raw(&url, &cfg) {
                let out_q_len = url::Url::parse(&out)
                    .ok()
                    .and_then(|u| u.query().map(|s| s.len()))
                    .unwrap_or(0);
                prop_assert!(out_q_len <= q.len(), "{out_q_len} > {len}", len = q.len());
            }
        }
    }

    #[test]
    fn session_keys_strip_not_drop() {
        let cfg = cfg_on();
        let out =
            filter_and_normalize_raw("https://legacy.example.com/page?jsessionid=ABC&p=1", &cfg)
                .unwrap();
        assert!(!out.contains("jsessionid"));
        assert!(out.contains("p=1"));
    }
}
