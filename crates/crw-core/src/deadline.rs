//! End-to-end deadline propagation across the scrape pipeline.
//!
//! A [`Deadline`] is constructed once at request entry from
//! `ScrapeRequest.deadline_ms` (falling back to `request.deadline_ms_default`)
//! and threaded through every layer that may sleep, retry, or wait — limiter
//! acquire, HTTP client timeouts, the chrome navigation budget, and post-extract
//! escalation. Each layer clamps its own timeout against [`Deadline::remaining`]
//! so the absolute return time is bounded by the original deadline.

use std::time::{Duration, Instant};

/// Absolute end-of-budget instant for a single request.
///
/// Cheap to copy. Compute remaining time with [`Self::remaining`]; never
/// schedule waits longer than that value.
#[derive(Debug, Clone, Copy)]
pub struct Deadline {
    absolute: Instant,
}

impl Deadline {
    /// Build a deadline `ms` milliseconds in the future, measured from now.
    /// `ms = 0` produces an immediately-expired deadline (useful in tests).
    pub fn from_request_ms(ms: u64) -> Self {
        Self {
            absolute: Instant::now() + Duration::from_millis(ms),
        }
    }

    /// Build a deadline `d` from now.
    pub fn now_plus(d: Duration) -> Self {
        Self {
            absolute: Instant::now() + d,
        }
    }

    /// Time remaining until the deadline. Returns `Duration::ZERO` if expired.
    pub fn remaining(&self) -> Duration {
        self.absolute.saturating_duration_since(Instant::now())
    }

    /// `true` once the deadline has passed.
    pub fn expired(&self) -> bool {
        Instant::now() >= self.absolute
    }

    /// How long ago the deadline expired. `Duration::ZERO` if not yet expired.
    /// Used to produce meaningful timeout error messages (vs. reporting 0ms).
    pub fn overrun(&self) -> Duration {
        Instant::now().saturating_duration_since(self.absolute)
    }

    /// The absolute wall-clock instant at which this deadline expires.
    pub fn absolute(&self) -> Instant {
        self.absolute
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_deadline_has_remaining() {
        let d = Deadline::from_request_ms(1000);
        assert!(d.remaining() > Duration::from_millis(900));
        assert!(!d.expired());
    }

    #[test]
    fn zero_ms_is_expired() {
        let d = Deadline::from_request_ms(0);
        assert!(d.expired());
        assert_eq!(d.remaining(), Duration::ZERO);
    }

    #[test]
    fn now_plus_matches_remaining() {
        let d = Deadline::now_plus(Duration::from_millis(500));
        assert!(d.remaining() > Duration::from_millis(400));
        assert!(d.remaining() <= Duration::from_millis(500));
    }
}
