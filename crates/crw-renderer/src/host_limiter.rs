//! Process-wide per-host rate-limiter and concurrency cap.
//!
//! Keyed by eTLD+1 (via [`crate::preference::normalize_host`]) so subdomains
//! under one registered domain share a single limiter. Each entry pairs a
//! [`RateLimiter`] (interval enforcement) with a [`Semaphore`] (in-flight
//! cap). Used by both [`crate::FallbackRenderer`] and the crawl/discover
//! loops in `crw-crawl` so single /scrape calls and crawl jobs respect the
//! same global per-host budget.
//!
//! First-write-wins for capacity and RPS — a second caller asking for
//! different values silently reuses the existing limiter (warned on RPS
//! mismatch). This matches the historical behaviour of the
//! `DOMAIN_RATE_LIMITERS` map this module replaces.

use dashmap::DashMap;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

/// Stale entries are dropped after this idle duration during periodic GC.
const RATE_LIMITER_TTL: Duration = Duration::from_secs(3600);

type RateLimiterEntry = (Arc<Mutex<RateLimiter>>, Arc<Semaphore>, Instant);

static DOMAIN_RATE_LIMITERS: LazyLock<DashMap<String, RateLimiterEntry>> =
    LazyLock::new(DashMap::new);

static LIMITER_CALL_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Get or create a (rate limiter, per-host semaphore) pair for the given
/// **already-normalized** host key. Pass the eTLD+1 form produced by
/// [`crate::preference::normalize_host`].
pub fn get_host_limiter(
    host_key: &str,
    rps: f64,
    per_host_max_concurrent: usize,
) -> (Arc<Mutex<RateLimiter>>, Arc<Semaphore>) {
    let now = Instant::now();
    if LIMITER_CALL_COUNT
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        .is_multiple_of(64)
    {
        DOMAIN_RATE_LIMITERS
            .retain(|_, (_, _, last_used)| now.duration_since(*last_used) < RATE_LIMITER_TTL);
    }
    let cap = per_host_max_concurrent.max(1);
    let entry = DOMAIN_RATE_LIMITERS
        .entry(host_key.to_string())
        .and_modify(|entry| {
            entry.2 = now;
            let existing_interval = entry.0.try_lock().map(|l| l.min_interval).ok();
            let new_interval = if rps > 0.0 {
                Duration::from_secs_f64(1.0 / rps)
            } else {
                Duration::ZERO
            };
            if let Some(existing) = existing_interval
                && existing != new_interval
            {
                tracing::warn!(
                    domain = host_key,
                    existing_rps = ?existing,
                    requested_rps = rps,
                    "Rate limiter RPS mismatch for domain; using existing limiter"
                );
            }
        })
        .or_insert_with(|| {
            (
                Arc::new(Mutex::new(RateLimiter::new(rps))),
                Arc::new(Semaphore::new(cap)),
                now,
            )
        });
    (entry.0.clone(), entry.1.clone())
}

/// Convenience: acquire the per-host concurrency permit and compute how long
/// the caller must sleep to honour the rate limit. The caller is responsible
/// for performing the sleep (so it doesn't happen while holding any other
/// async lock) and for keeping the permit alive across the request.
pub async fn acquire(
    host_key: &str,
    rps: f64,
    per_host_max_concurrent: usize,
) -> Result<(OwnedSemaphorePermit, Duration), tokio::sync::AcquireError> {
    let (rate_limiter, sem) = get_host_limiter(host_key, rps, per_host_max_concurrent);
    let permit = sem.acquire_owned().await?;
    let sleep = rate_limiter.lock().await.next_sleep();
    Ok((permit, sleep))
}

/// Minimum-interval rate limiter. Public so `crw-crawl` can keep its existing
/// jitter wrapper.
pub struct RateLimiter {
    pub min_interval: Duration,
    last_request: Instant,
}

impl RateLimiter {
    pub fn new(requests_per_second: f64) -> Self {
        if requests_per_second < 0.0 {
            tracing::warn!(
                requests_per_second,
                "Negative requests_per_second value, treating as unlimited"
            );
        }
        let min_interval = if requests_per_second > 0.0 {
            Duration::from_secs_f64(1.0 / requests_per_second)
        } else {
            Duration::ZERO
        };
        Self {
            min_interval,
            last_request: Instant::now() - min_interval,
        }
    }

    /// Compute how long to sleep and update last_request. Caller must sleep
    /// outside any held async lock.
    pub fn next_sleep(&mut self) -> Duration {
        let elapsed = self.last_request.elapsed();
        let sleep = if elapsed < self.min_interval {
            self.min_interval - elapsed
        } else {
            Duration::ZERO
        };
        self.last_request = Instant::now() + sleep;
        sleep
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limiter_zero_rps_is_unlimited() {
        let mut limiter = RateLimiter::new(0.0);
        assert_eq!(limiter.next_sleep(), Duration::ZERO);
        assert_eq!(limiter.next_sleep(), Duration::ZERO);
    }

    #[test]
    fn rate_limiter_negative_rps_is_unlimited() {
        let mut limiter = RateLimiter::new(-1.0);
        assert_eq!(limiter.next_sleep(), Duration::ZERO);
    }

    #[test]
    fn rate_limiter_enforces_interval() {
        let mut limiter = RateLimiter::new(10.0); // 100ms interval
        let _first = limiter.next_sleep();
        let second = limiter.next_sleep();
        assert!(second > Duration::from_millis(50) && second <= Duration::from_millis(100));
    }

    #[tokio::test]
    async fn acquire_returns_permit_and_sleep() {
        let (_p, sleep) = acquire("test-acquire-1.example", 0.0, 1).await.unwrap();
        assert_eq!(sleep, Duration::ZERO);
    }

    #[tokio::test]
    async fn per_host_cap_serializes() {
        let (p1, _) = acquire("test-cap.example", 0.0, 1).await.unwrap();
        // Second acquire must wait — verify it doesn't complete in 50ms.
        let acquire_fut = acquire("test-cap.example", 0.0, 1);
        let race = tokio::time::timeout(Duration::from_millis(50), acquire_fut).await;
        assert!(race.is_err(), "second acquire should block while p1 held");
        drop(p1);
        // Now it should succeed quickly.
        let (_p2, _) = tokio::time::timeout(
            Duration::from_millis(200),
            acquire("test-cap.example", 0.0, 1),
        )
        .await
        .expect("second acquire should succeed after release")
        .unwrap();
    }
}
