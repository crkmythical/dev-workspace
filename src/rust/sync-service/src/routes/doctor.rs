//! GET /sync/api/doctor — port of packages/server/src/routes/doctor.ts.
//!
//! Always 200. A disabled feature is reported as "disabled", not as a locked
//! vault. tests/smoke/checks/20-auth.sh pins the JSON shape.

use axum::{extract::State, response::IntoResponse, Json};
use serde_json::json;

use crate::AppState;

pub async fn doctor(State(state): State<AppState>) -> impl IntoResponse {
    let enabled = state.features.sync;
    let has_key = state.sync_key_available();
    let st = if !enabled {
        "disabled"
    } else if has_key {
        "ready"
    } else {
        "vault-locked"
    };
    Json(json!({
        "state": st,
        "sync_enabled": enabled,
        "sync_key_available": has_key,
    }))
}
