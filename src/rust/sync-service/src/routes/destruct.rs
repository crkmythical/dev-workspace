//! POST /sync/api/destruct — port of packages/server/src/routes/destruct.ts.
//!
//! Off unless REMOTE_DESTRUCT_ENABLED. When off, every call is a fast 404, indistinguishable from
//! an endpoint that does not exist. When on, refusals are 404 too (wrong or empty passphrase, no
//! key hash) — with two caveats stated honestly rather than claimed away: a sustained flood trips
//! the per-IP budget / global backstop and gets a 429 (not a 404), and only a well-formed wrong
//! passphrase reaches the deliberately slow bcrypt verify. So response timing and the 429 can
//! reveal that the feature is enabled, though neither leaks the passphrase. On the right passphrase
//! it launches the destroyer synchronously and answers 200 — or 500 if it could not even start, so
//! a launch failure is never hidden behind an optimistic 200.

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};

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

    // Launch the destroyer and only answer success if it actually started. A failure to even
    // spawn it must not be hidden behind a 200. The child does the slow, irreversible work
    // (quiesce, unmount, shred, then SIGTERM pid 1) — far longer than this tiny JSON needs to
    // flush back through Caddy, so there is no race that "answer first" was protecting against
    // (quiesce stops only the vault-sync cron, not Caddy or this daemon).
    match state.destruct.trigger() {
        Ok(()) => (StatusCode::OK, Json(json!({ "status": "destroyed" }))).into_response(),
        Err(e) => {
            eprintln!("remote destruct: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "status": "error" })),
            )
                .into_response()
        }
    }
}
