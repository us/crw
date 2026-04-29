//! Per-host renderer preference learning.
//!
//! Tracks a sliding window of LightPanda failures per normalized host and
//! promotes the host to a heavier renderer (Chrome) when the failure rate
//! crosses a threshold. The cache is bounded by entry count and entries
//! expire on idle to keep memory predictable.
//!
//! ## Failure semantics
//!
//! Only LightPanda-specific failures count toward promotion (see
//! [`FailoverErrorKind::counts_for_promotion`]). Cloudflare challenges,
//! network errors, and "other" failures are recorded but do not drive
//! promotion — that's the strict-predicate guard from the plan review.
//!
//! ## Concurrency
//!
//! Each host's stats live behind a single `Mutex` to avoid TOCTOU races
//! between `record_failure` and `should_promote`. The cache itself is
//! `moka` async, lock-free for reads.

use crw_core::types::{FailoverErrorKind, RendererKind};
use moka::future::Cache;
use publicsuffix::{List, Psl};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Maximum number of failures we remember per host (sliding window cap).
const WINDOW_CAP: usize = 32;

/// Sliding window length — failures older than this are discarded.
const WINDOW_DURATION: Duration = Duration::from_secs(15 * 60);

/// Default cache capacity (number of distinct hosts tracked).
pub const DEFAULT_CAPACITY: u64 = 10_000;

/// Default idle TTL: hosts unused for this long are evicted.
pub const DEFAULT_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Failures within the sliding window required before promoting a host.
const PROMOTION_THRESHOLD: usize = 3;

#[derive(Debug)]
struct WindowEntry {
    at: Instant,
    /// Whether this failure counts toward promotion (strict predicate).
    counts: bool,
}

/// Per-host failure state. Single Mutex protects the entire view to avoid
/// races between observation and decision.
#[derive(Debug, Default)]
pub struct RendererStats {
    inner: Mutex<StatsInner>,
}

#[derive(Debug, Default)]
struct StatsInner {
    failures: VecDeque<WindowEntry>,
    /// Whether this host has already been promoted (latched until reset).
    promoted: bool,
}

impl RendererStats {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a failure observed against this host. Returns `true` if this
    /// call caused a promotion transition (counter crossed the threshold).
    pub fn record_failure(&self, kind: &FailoverErrorKind) -> bool {
        let counts = kind.counts_for_promotion();
        let now = Instant::now();
        let mut inner = self.inner.lock().expect("RendererStats mutex poisoned");

        // Drop expired entries.
        while let Some(front) = inner.failures.front() {
            if now.duration_since(front.at) > WINDOW_DURATION {
                inner.failures.pop_front();
            } else {
                break;
            }
        }
        if inner.failures.len() >= WINDOW_CAP {
            inner.failures.pop_front();
        }
        inner.failures.push_back(WindowEntry { at: now, counts });

        if inner.promoted {
            return false;
        }
        let counting: usize = inner.failures.iter().filter(|e| e.counts).count();
        if counting >= PROMOTION_THRESHOLD {
            inner.promoted = true;
            true
        } else {
            false
        }
    }

    /// Record a successful render — clears the promotion latch and trims
    /// half the window so a recovered host can return to LightPanda.
    pub fn record_success(&self) {
        let mut inner = self.inner.lock().expect("RendererStats mutex poisoned");
        inner.promoted = false;
        let drop_n = inner.failures.len() / 2;
        for _ in 0..drop_n {
            inner.failures.pop_front();
        }
    }

    /// True if this host is currently promoted to a heavier renderer.
    pub fn is_promoted(&self) -> bool {
        self.inner
            .lock()
            .expect("RendererStats mutex poisoned")
            .promoted
    }
}

/// Per-host renderer preference cache. Cheap to clone (`Arc` inside).
#[derive(Clone)]
pub struct HostPreferences {
    cache: Cache<String, Arc<RendererStats>>,
}

impl HostPreferences {
    pub fn new(capacity: u64, ttl: Duration) -> Self {
        let cache = Cache::builder()
            .max_capacity(capacity)
            .time_to_idle(ttl)
            .build();
        Self { cache }
    }

    pub fn with_defaults() -> Self {
        Self::new(DEFAULT_CAPACITY, DEFAULT_TTL)
    }

    async fn stats_for(&self, host: &str) -> Arc<RendererStats> {
        let key = host.to_string();
        self.cache
            .get_with(key, async { Arc::new(RendererStats::new()) })
            .await
    }

    /// Record a failure for `host` (will be normalized). Returns the
    /// promotion target if this call promoted the host, else `None`.
    pub async fn record_failure(
        &self,
        host: &str,
        kind: &FailoverErrorKind,
    ) -> Option<RendererKind> {
        let normalized = normalize_host(host);
        let stats = self.stats_for(&normalized).await;
        if stats.record_failure(kind) {
            Some(RendererKind::Chrome)
        } else {
            None
        }
    }

    /// Record a successful render for `host` (will be normalized).
    pub async fn record_success(&self, host: &str) {
        let normalized = normalize_host(host);
        let stats = self.stats_for(&normalized).await;
        stats.record_success();
    }

    /// Returns the preferred renderer for `host` if a promotion is in
    /// effect, else `None` (caller falls back to default chain).
    pub async fn preferred(&self, host: &str) -> Option<RendererKind> {
        let normalized = normalize_host(host);
        let stats = self.cache.get(&normalized).await?;
        if stats.is_promoted() {
            Some(RendererKind::Chrome)
        } else {
            None
        }
    }

    /// Clear all preference state.
    pub async fn reset_all(&self) {
        self.cache.invalidate_all();
        self.cache.run_pending_tasks().await;
    }

    /// Clear preference state for a specific host (will be normalized).
    pub async fn reset_host(&self, host: &str) {
        let normalized = normalize_host(host);
        self.cache.invalidate(&normalized).await;
    }

    /// Current cache size (approximate).
    pub fn size(&self) -> u64 {
        self.cache.entry_count()
    }
}

impl Default for HostPreferences {
    fn default() -> Self {
        Self::with_defaults()
    }
}

// ── Host normalization ────────────────────────────────────────────────

static PSL: OnceLock<List> = OnceLock::new();

fn psl() -> &'static List {
    PSL.get_or_init(|| {
        // Embedded snapshot ships with publicsuffix.
        include_str!("public_suffix_list.dat")
            .parse()
            .expect("embedded PSL must parse")
    })
}

/// Normalize a host for cache keying:
/// - lowercase
/// - strip leading `www.`
/// - collapse to registrable domain (eTLD+1) using the public suffix list
///
/// Multi-tenant hosts under a public suffix (e.g. `foo.myshopify.com`,
/// `foo.vercel.app`) keep their tenant label because the suffix itself
/// is `myshopify.com` / `vercel.app` — eTLD+1 ends up being the tenant.
pub fn normalize_host(input: &str) -> String {
    let lower = input.trim().to_ascii_lowercase();
    let trimmed = lower.strip_prefix("www.").unwrap_or(&lower);
    let bytes = trimmed.as_bytes();
    match psl().domain(bytes) {
        Some(domain) => std::str::from_utf8(domain.as_bytes())
            .unwrap_or(trimmed)
            .to_string(),
        None => trimmed.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_www_prefix() {
        assert_eq!(normalize_host("www.example.com"), "example.com");
    }

    #[test]
    fn keeps_shopify_tenant() {
        assert_eq!(normalize_host("foo.myshopify.com"), "foo.myshopify.com");
    }

    #[test]
    fn keeps_vercel_tenant() {
        assert_eq!(normalize_host("myapp.vercel.app"), "myapp.vercel.app");
    }

    #[test]
    fn collapses_subdomains_to_registrable() {
        assert_eq!(normalize_host("a.b.example.com"), "example.com");
    }

    #[test]
    fn handles_co_uk_etld() {
        assert_eq!(normalize_host("www.example.co.uk"), "example.co.uk");
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(normalize_host("WWW.Example.COM"), "example.com");
    }

    #[test]
    fn renderer_stats_promotes_on_threshold() {
        let stats = RendererStats::new();
        assert!(!stats.record_failure(&FailoverErrorKind::NextJsClientError));
        assert!(!stats.record_failure(&FailoverErrorKind::EmptyNextRoot));
        assert!(stats.record_failure(&FailoverErrorKind::LightpandaTimeout));
        assert!(stats.is_promoted());
    }

    #[test]
    fn renderer_stats_strict_predicate_excludes_cf() {
        let stats = RendererStats::new();
        for _ in 0..5 {
            stats.record_failure(&FailoverErrorKind::CloudflareChallenge);
        }
        assert!(!stats.is_promoted());
    }

    #[test]
    fn renderer_stats_success_clears_promotion() {
        let stats = RendererStats::new();
        for _ in 0..3 {
            stats.record_failure(&FailoverErrorKind::NextJsClientError);
        }
        assert!(stats.is_promoted());
        stats.record_success();
        assert!(!stats.is_promoted());
    }

    #[test]
    fn renderer_stats_window_capped() {
        let stats = RendererStats::new();
        for _ in 0..(WINDOW_CAP + 10) {
            stats.record_failure(&FailoverErrorKind::Other);
        }
        let inner = stats.inner.lock().unwrap();
        assert!(inner.failures.len() <= WINDOW_CAP);
    }

    #[tokio::test]
    async fn host_preferences_promotes_after_threshold() {
        let prefs = HostPreferences::with_defaults();
        for kind in [
            FailoverErrorKind::NextJsClientError,
            FailoverErrorKind::EmptyNextRoot,
        ] {
            assert_eq!(prefs.record_failure("example.com", &kind).await, None);
        }
        assert_eq!(
            prefs
                .record_failure("example.com", &FailoverErrorKind::LightpandaTimeout)
                .await,
            Some(RendererKind::Chrome)
        );
        assert_eq!(
            prefs.preferred("example.com").await,
            Some(RendererKind::Chrome)
        );
    }

    #[tokio::test]
    async fn host_preferences_normalize_collapses_subdomain() {
        let prefs = HostPreferences::with_defaults();
        for _ in 0..3 {
            prefs
                .record_failure("a.b.example.com", &FailoverErrorKind::NextJsClientError)
                .await;
        }
        assert_eq!(
            prefs.preferred("www.example.com").await,
            Some(RendererKind::Chrome)
        );
    }

    #[tokio::test]
    async fn host_preferences_reset_clears_state() {
        let prefs = HostPreferences::with_defaults();
        for _ in 0..3 {
            prefs
                .record_failure("example.com", &FailoverErrorKind::NextJsClientError)
                .await;
        }
        prefs.reset_all().await;
        assert_eq!(prefs.preferred("example.com").await, None);
    }
}
