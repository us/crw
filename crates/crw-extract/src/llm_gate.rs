//! Process-wide reserved concurrency gate for LLM provider calls (the D lane).
//!
//! Every LLM HTTP call — fallback extraction, structured JSON, summary, and
//! change-tracking judge — funnels through the two lowest provider-call
//! boundaries ([`crate::llm::dispatch`] and [`crate::structured::call_anthropic`]
//! / [`call_openai`]), which acquire a permit here. LLM calls run on the async
//! path (no `spawn_blocking`), so the [`crw_core::REQUEST_CLASS`] task-local is
//! read directly at the call site.
//!
//! Before this gate the advertised `extraction.llm.max_concurrency` knob was
//! dead code (only echoed in `/v1/capabilities`); wiring it here both bounds LLM
//! fan-out and reserves a lane so a batch's `formats:["json"]` fan-out can't
//! starve interactive LLM requests via provider-side rate-limiting/latency.

use std::sync::OnceLock;

use crw_core::{LanePermit, ReservedSemaphore, current_scrape_class};

static LLM_SEM: OnceLock<ReservedSemaphore> = OnceLock::new();

/// Install the LLM concurrency cap with `reserved` permits held for interactive
/// traffic. Idempotent — the first call wins, so call it once at boot from
/// `extraction.llm.{max_concurrency, reserved_interactive_llm}`.
pub fn configure_llm_limits(total: usize, reserved: usize) {
    let _ = LLM_SEM.set(ReservedSemaphore::new(total, reserved, "llm"));
}

fn gate() -> &'static ReservedSemaphore {
    // Fallback for surfaces that never configure (CLI, tests): default total 4,
    // reserve 1, matching `default_llm_max_concurrency` / its reserve.
    LLM_SEM.get_or_init(|| ReservedSemaphore::new(4, 1, "llm"))
}

/// Acquire the LLM-call lane for the current traffic class. Hold the returned
/// permit across the provider HTTP call. MUST be called on the async side.
pub async fn acquire_llm() -> LanePermit {
    gate().acquire(current_scrape_class()).await
}
