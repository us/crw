//! Self-host monitor-mode boot hook (feature `monitor`, default OFF).
//!
//! Constructs the `crw-monitor` SQLite store + an engine-backed page source
//! from the already-built [`AppState`], then spawns the background scheduler.
//! All monitor endpoints/scheduling live behind `#[cfg(feature = "monitor")]`
//! so the default open-core build never links the SQLite/HMAC/cron stack.
//!
//! ponytail: `boot()` is NOT called from server startup today — it is the
//! wiring stub for a deferred self-host feature (no HTTP/CLI surface, email is
//! an `EmailStub`). A `--features monitor` build links the library but does not
//! run a scheduler until something calls this. Wire it to a config toggle +
//! a minimal `/v1/monitor` route only when a self-hoster actually asks. See #142.

use crate::state::AppState;
use crw_monitor::config::MonitorConfig;
use crw_monitor::runner::EngineSource;
use crw_monitor::{Scheduler, Store};
use std::sync::Arc;

/// Boot the self-host monitor scheduler. Returns the spawned task handle (or an
/// error if the store cannot be opened). The caller decides whether to keep it.
pub fn boot(state: &AppState, cfg: MonitorConfig) -> Result<tokio::task::JoinHandle<()>, String> {
    let store = Store::open(&cfg.db_path).map_err(|e| e.to_string())?;
    let store = Arc::new(store);

    let source = Arc::new(EngineSource::new(
        state.config.clone(),
        state.renderer.clone(),
        &cfg,
    ));

    let default_llm = state.config.extraction.llm.clone();
    let scheduler = Scheduler::new(store, source, cfg, default_llm);
    Ok(scheduler.spawn())
}
