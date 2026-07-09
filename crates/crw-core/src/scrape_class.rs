//! Request traffic class — `Interactive` (single `/scrape`) vs `Batch`
//! (batch-scrape / crawl jobs) — carried via a task-local so every shared
//! concurrency chokepoint (per-host limiter, render pool, extract pool, PDF
//! parse pool, LLM calls) can reserve a protected lane for interactive traffic
//! without threading a parameter through dozens of signatures.
//!
//! Lives in `crw-core` (not `crw-renderer`) deliberately: `crw-renderer`
//! depends on `crw-extract`, so an LLM-lane read of a `crw-renderer`-owned
//! task-local would form a dependency cycle. `crw-core` is a dependency of all
//! three, so every lane can read it.
//!
//! The class is set by the JOB ENTRY POINT, not the wire request — there is no
//! deserialized field a client could set to jump the interactive reserve. A
//! single scrape leaves the task-local unscoped and reads back `Interactive`
//! via [`current_scrape_class`]'s default; batch/crawl jobs wrap their work in
//! `REQUEST_CLASS.scope(ScrapeClass::Batch, …)` INSIDE the spawned task (a
//! handler-level scope would be lost across the job's `tokio::spawn`).

/// Traffic class for a scrape. `Interactive` is the protected default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrapeClass {
    /// A single interactive `/scrape` request. Gets the reserved lane at every
    /// shared chokepoint.
    #[default]
    Interactive,
    /// A URL inside a batch-scrape or crawl job. Uses the batch lane, which can
    /// never consume the interactive reserve.
    Batch,
}

impl ScrapeClass {
    /// `true` for [`ScrapeClass::Batch`].
    pub fn is_batch(self) -> bool {
        matches!(self, ScrapeClass::Batch)
    }
}

tokio::task_local! {
    /// The current request's traffic class. Scoped by batch/crawl job entry
    /// points; unscoped for single scrapes (reads back `Interactive`). Read via
    /// [`current_scrape_class`] — do NOT `try_with` it inside a `spawn_blocking`
    /// closure, task-locals do not cross that boundary; read it on the async
    /// side before spawning the blocking work.
    pub static REQUEST_CLASS: ScrapeClass;
}

/// The current task's [`ScrapeClass`], or [`ScrapeClass::Interactive`] when the
/// task-local is not in scope (every direct single-scrape caller). MUST be
/// called on the async side, before any `spawn_blocking` — the task-local is
/// not visible inside a blocking closure.
pub fn current_scrape_class() -> ScrapeClass {
    REQUEST_CLASS.try_with(|c| *c).unwrap_or_default()
}
