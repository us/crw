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

use std::sync::{Arc, OnceLock};

use tokio::sync::Semaphore;

use crw_core::error::{CrwError, CrwResult};
use crw_core::types::ScrapeData;
use crw_extract::OwnedExtractInput;

/// Process-wide cap on concurrent HTML extractions. Installed once at startup
/// via [`configure_extract_limit`]; surfaces that never configure (CLI, tests)
/// fall back to [`default_extract_limit`].
static EXTRACT_SEM: OnceLock<Arc<Semaphore>> = OnceLock::new();

/// Fallback bound when [`configure_extract_limit`] is never called: ~2/3 of the
/// available cores, floored at 2. Leaves headroom for the async reactor and
/// renderer threads instead of letting extraction claim every core.
fn default_extract_limit() -> usize {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    (cpus * 2 / 3).max(2)
}

/// Install the concurrent-extraction cap. Idempotent — the first call wins
/// (subsequent calls are ignored), so call it once at boot. Mirrors
/// [`crate::pdf::configure_limits`].
pub fn configure_extract_limit(n: usize) {
    let _ = EXTRACT_SEM.set(Arc::new(Semaphore::new(n.max(1))));
}

fn sem() -> &'static Arc<Semaphore> {
    EXTRACT_SEM.get_or_init(|| Arc::new(Semaphore::new(default_extract_limit())))
}

/// Run one HTML → markdown extraction off the async reactor.
///
/// Acquires a concurrency permit (held for the whole CPU job, even if the
/// caller is dropped early), then runs [`crw_extract::extract`] on the blocking
/// pool. This keeps the reactor responsive and bounds peak CPU parallelism to
/// the configured limit so a burst of concurrent scrapes can't stall the
/// runtime.
pub async fn extract_offloaded(input: OwnedExtractInput) -> CrwResult<ScrapeData> {
    // Acquire BEFORE spawning and move the permit into the closure so it is held
    // for the full duration of the blocking parse — keeping the semaphore an
    // honest bound on real concurrent CPU work.
    let permit = sem()
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| CrwError::ExtractionError("extract semaphore closed".to_string()))?;

    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        crw_extract::extract(input.as_opts())
    })
    .await
    .map_err(|e| CrwError::ExtractionError(format!("extract task failed: {e}")))?
}
