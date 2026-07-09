//! Off-reactor, parallelism-bounded HTML → markdown extraction.
//!
//! `crw_extract::extract` (html5ever + htmd) is CPU-bound and synchronous.
//! Running it inline on the tokio worker threads starves the async reactor
//! under high scrape concurrency (~600 concurrent scrapes), collapsing
//! throughput. This module moves each extraction onto the blocking pool via
//! [`tokio::task::spawn_blocking`] and bounds the number of concurrent
//! extractions with a process-wide [`Semaphore`] so the CPU work can't
//! oversubscribe the cores. The runtime builder is left untouched
//! (`worker_threads`/`max_blocking_threads` at their defaults) — this semaphore
//! is the real bound. Mirrors the PDF parse gate in [`crate::pdf`].

use std::sync::OnceLock;

use crw_core::error::{CrwError, CrwResult};
use crw_core::types::ScrapeData;
use crw_core::{ReservedSemaphore, current_scrape_class};
use crw_extract::OwnedExtractInput;

/// Process-wide reserved cap on concurrent HTML extractions. Installed once at
/// startup via [`configure_extract_limit`]; surfaces that never configure (CLI,
/// tests) fall back to [`default_extract_limit`]. The reserve guarantees
/// interactive single-scrape extractions a slice of CPU permits that batch/crawl
/// extractions can never hold (see [`ReservedSemaphore`]).
static EXTRACT_SEM: OnceLock<ReservedSemaphore> = OnceLock::new();

/// Fallback bound when [`configure_extract_limit`] is never called: ~2/3 of the
/// available cores, floored at 2. Leaves headroom for the async reactor and
/// renderer threads instead of letting extraction claim every core.
fn default_extract_limit() -> usize {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    (cpus * 2 / 3).max(2)
}

/// Default interactive reserve: a quarter of the pool, floored at 1. `0`
/// disables the reserve (single-lane FIFO, legacy behaviour).
fn default_reserve(total: usize) -> usize {
    (total / 4).max(1)
}

/// Install the concurrent-extraction cap with `reserved` permits held for
/// interactive traffic. Idempotent — the first call wins (subsequent calls are
/// ignored), so call it once at boot. Mirrors [`crate::pdf::configure_limits`].
pub fn configure_extract_limit(total: usize, reserved: usize) {
    let _ = EXTRACT_SEM.set(ReservedSemaphore::new(total, reserved, "extract"));
}

fn sem() -> &'static ReservedSemaphore {
    EXTRACT_SEM.get_or_init(|| {
        let total = default_extract_limit();
        ReservedSemaphore::new(total, default_reserve(total), "extract")
    })
}

/// Run one HTML → markdown extraction off the async reactor.
///
/// Reads the traffic class and acquires the matching reserved lane BEFORE
/// spawning — the [`crw_core::REQUEST_CLASS`] task-local is not visible inside
/// the blocking closure, so lane selection must happen on the async side. The
/// permit is moved into the closure and held for the whole CPU job.
///
/// `ponytail:` the parse has no internal deadline — a caller-side timeout (see
/// the callers) unblocks the awaiting task but does NOT cancel this
/// `spawn_blocking`, so a pathological document holds its permit until the parse
/// returns naturally. Ceiling = worst-case single-document parse time; upgrade
/// path = a size/complexity pre-check before spawn_blocking, or a
/// cancellable/process-isolated parser.
pub async fn extract_offloaded(input: OwnedExtractInput) -> CrwResult<ScrapeData> {
    let permit = sem().acquire(current_scrape_class()).await;

    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        crw_extract::extract(input.as_opts())
    })
    .await
    .map_err(|e| CrwError::ExtractionError(format!("extract task failed: {e}")))?
}
