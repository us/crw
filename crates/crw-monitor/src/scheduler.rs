//! Background tokio scheduler: ticks on an interval, finds due monitors, runs
//! their checks, persists results, advances schedules, and fires webhooks.
//!
//! UTC-only. The tick loop is a simple `tokio::time::interval` rather than an
//! external cron scheduler — deterministic and dependency-light. Each monitor's
//! own `schedule` string is parsed per-tick to compute its `next_run_at`.

#[cfg(feature = "store")]
use crate::config::MonitorConfig;
#[cfg(feature = "store")]
use crate::runner::{EngineSource, run_check};
#[cfg(feature = "store")]
use crate::schedule::Schedule;
#[cfg(feature = "store")]
use crate::store::Store;
#[cfg(feature = "store")]
use crate::types::Monitor;
#[cfg(feature = "store")]
use crw_core::config::LlmConfig;
#[cfg(feature = "store")]
use std::sync::Arc;

/// The self-host scheduler. Owns the [`Store`] + an [`EngineSource`] and runs a
/// background tick loop until dropped/aborted.
#[cfg(feature = "store")]
pub struct Scheduler {
    store: Arc<Store>,
    source: Arc<EngineSource>,
    cfg: MonitorConfig,
    http: reqwest::Client,
    /// Server-level default LLM config used for judging when a monitor doesn't
    /// supply its own BYOK key.
    default_llm: Option<LlmConfig>,
}

#[cfg(feature = "store")]
impl Scheduler {
    pub fn new(
        store: Arc<Store>,
        source: Arc<EngineSource>,
        cfg: MonitorConfig,
        default_llm: Option<LlmConfig>,
    ) -> Self {
        Self {
            store,
            source,
            cfg,
            http: reqwest::Client::new(),
            default_llm,
        }
    }

    /// Spawn the tick loop as a background task. Returns its [`JoinHandle`].
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move { self.run().await })
    }

    /// Run the tick loop forever.
    pub async fn run(self) {
        let mut ticker =
            tokio::time::interval(std::time::Duration::from_secs(self.cfg.tick_secs.max(1)));
        loop {
            ticker.tick().await;
            if let Err(e) = self.tick().await {
                tracing::error!(error = %e, "monitor scheduler tick failed");
            }
        }
    }

    /// One tick: run every due monitor once.
    pub async fn tick(&self) -> crate::MonitorResult<()> {
        let now = now_unix();
        let due = self.store.due_monitors(now)?;
        for monitor in due {
            if let Err(e) = self.run_monitor(&monitor, now).await {
                tracing::error!(monitor = %monitor.id, error = %e, "monitor check failed");
            }
        }
        Ok(())
    }

    /// Run all targets of one monitor, persist, advance schedule, fire webhook.
    pub async fn run_monitor(&self, monitor: &Monitor, now: i64) -> crate::MonitorResult<()> {
        let targets = self.store.get_targets(&monitor.id)?;
        let judge_llm = self.resolve_judge_llm(monitor);

        for target in &targets {
            let prior = self.store.load_prior(&monitor.id)?;
            // Monitor checks are background work — `Batch` traffic, so their
            // fetches and LLM judge calls use the batch lanes and never consume
            // the interactive reserve.
            let check = crw_core::REQUEST_CLASS
                .scope(
                    crw_core::ScrapeClass::Batch,
                    run_check(
                        monitor,
                        target,
                        &prior,
                        self.source.as_ref(),
                        &self.cfg,
                        judge_llm.as_ref(),
                        now,
                    ),
                )
                .await?;

            // Persist the check (also advances snapshot baselines).
            self.store.record_check(&check)?;

            // Fire webhook (best-effort).
            if let Some(webhook) = &monitor.webhook
                && let Err(e) = crate::webhook::deliver(&self.http, webhook, &check).await
            {
                tracing::warn!(monitor = %monitor.id, error = %e, "webhook delivery failed");
            }
        }

        // Advance the schedule cursor.
        match Schedule::parse(&monitor.schedule) {
            Ok(sched) => {
                let next = sched.next_after(now);
                self.store.update_schedule(&monitor.id, now, next)?;
            }
            Err(e) => {
                tracing::error!(monitor = %monitor.id, error = %e, "invalid schedule; pausing");
                self.store
                    .set_status(&monitor.id, crate::types::MonitorStatus::Paused)?;
            }
        }
        Ok(())
    }

    /// Pick the LLM config for judging: per-monitor BYOK wins, else the server
    /// default. Returns `None` (judging disabled) when neither has a key.
    fn resolve_judge_llm(&self, monitor: &Monitor) -> Option<LlmConfig> {
        if !monitor.judge_enabled || monitor.goal.is_none() {
            return None;
        }
        if let Some(key) = &monitor.llm_api_key
            && !key.is_empty()
        {
            let base = self.default_llm.clone().unwrap_or_default();
            return Some(LlmConfig {
                provider: monitor
                    .llm_provider
                    .clone()
                    .unwrap_or(base.provider.clone()),
                api_key: key.clone(),
                model: monitor.llm_model.clone().unwrap_or(base.model.clone()),
                ..base
            });
        }
        // Fall back to the server's own key (operator-owned).
        self.default_llm.clone().filter(|l| !l.api_key.is_empty())
    }
}

#[cfg(feature = "store")]
fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
