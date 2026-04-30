use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;

use crate::state::AppState;

/// `GET /metrics/renderer-breakers` — surfaces the current state of every
/// global and per-host circuit breaker so operators can tell at a glance
/// which renderer/host pairs are currently shedding load.
pub async fn renderer_breakers(State(state): State<AppState>) -> impl IntoResponse {
    Json(state.renderer.breakers().snapshot())
}
