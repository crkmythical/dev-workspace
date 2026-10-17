//! GET /sync/api/doctor — minimal liveness + vault state.

use axum::{extract::State, response::IntoResponse, Json};
use serde_json::json;

use crate::AppState;

pub async fn doctor(State(state): State<AppState>) -> impl IntoResponse {
    let unlocked = state.sync_key().is_some();
    Json(json!({
        "service": "sync-service-rs",
        "vault": if unlocked { "unlocked" } else { "locked" },
    }))
}
