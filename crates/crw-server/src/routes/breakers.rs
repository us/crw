use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use serde_json::json;

use crate::state::AppState;

/// `GET /metrics/renderer-breakers` — surfaces the current state of every
/// global and per-host circuit breaker so operators can tell at a glance
/// which renderer/host pairs are currently shedding load.
pub async fn renderer_breakers(State(state): State<AppState>) -> impl IntoResponse {
    Json(state.renderer.breakers().snapshot())
}

/// `POST /admin/breakers/reset` — slams every global breaker back to Closed
/// and evicts the per-host cache. Returns the count of host entries cleared.
/// Use after an operational incident (LP/Chrome restart, image rollback) to
/// abort the cooldown cycle without waiting for max_cooldown.
pub async fn reset_breakers(State(state): State<AppState>) -> impl IntoResponse {
    let cleared = state.renderer.breakers().reset_all();
    tracing::warn!(cleared, "admin: all renderer breakers reset");
    Json(json!({ "ok": true, "host_entries_cleared": cleared }))
}
