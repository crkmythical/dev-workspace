//! POST /sync/api/destruct — port of packages/server/src/routes/destruct.ts.
//!
//! Off unless REMOTE_DESTRUCT_ENABLED, and then every refusal is a 404, so a
//! disabled or wrongly-answered endpoint is indistinguishable from one that does
//! not exist. A per-IP budget with a global backstop bounds a brute force. On the
//! right passphrase it answers 200 and, after a short delay, fires the trigger.

use std::time::Duration;

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};

use sdw_core::vault::destruct::DESTRUCT_RESPONSE_DELAY_MS;

use crate::util::now_ms;
use crate::AppState;

/// Prefer Cloudflare's edge-set CF-Connecting-IP: when the only ingress is the CF
/// tunnel, an external client cannot overwrite it. Fall back to the first
/// X-Forwarded-For hop, then a constant (which still shares the global cap).
fn client_ip(headers: &HeaderMap) -> String {
    if let Some(cf) = headers
        .get("cf-connecting-ip")
        .and_then(|v| v.to_str().ok())
    {
        return cf.trim().to_string();
    }
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first) = xff.split(',').next() {
            return first.trim().to_string();
        }
    }
    "unknown".to_string()
}

pub async fn destruct(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    // Checked before anything else, so a disabled endpoint is indistinguishable
    // from a missing one even if a stale key hash is still on the volume.
    if !state.features.remote_destruct {
        return StatusCode::NOT_FOUND.into_response();
    }

    let ip = client_ip(&headers);
    if state.limiter.lock().unwrap().check(&ip, now_ms()) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }

    // The key hash exists only when the vault was unlocked with the feature on.
    if !std::path::Path::new(&*state.key_hash_path).exists() {
        return StatusCode::NOT_FOUND.into_response();
    }

    // Body is { "passphrase": "..." }. Anything else is a 404.
    let passphrase = match serde_json::from_slice::<Value>(&body) {
        Ok(v) => v
            .get("passphrase")
            .and_then(Value::as_str)
            .map(str::to_string),
        Err(_) => None,
    };
    let Some(passphrase) = passphrase.filter(|p| !p.is_empty()) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let Ok(hash) = std::fs::read_to_string(&*state.key_hash_path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let hash = hash.trim().to_string();

    // bcrypt is deliberately slow; keep it off the runtime thread.
    let valid = tokio::task::spawn_blocking(move || bcrypt::verify(&passphrase, &hash))
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or(false);
    if !valid {
        return StatusCode::NOT_FOUND.into_response();
    }

    // Answer first, destroy after a short delay (the TS route's setTimeout). The
    // trigger owns the irreversible path and terminates PID 1.
    let trigger = state.destruct.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(DESTRUCT_RESPONSE_DELAY_MS)).await;
        trigger.trigger();
    });

    (StatusCode::OK, Json(json!({ "status": "destroyed" }))).into_response()
}
