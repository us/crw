use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use crate::state::AppState;

pub async fn health(State(state): State<AppState>) -> Json<Value> {
    let renderer_health = state.renderer.check_health().await;
    let jobs = state.crawl_jobs.read().await;

    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "renderers": renderer_health,
        "active_crawl_jobs": jobs.len(),
    }))
}
