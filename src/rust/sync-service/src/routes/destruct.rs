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
//!
//! Every attempt that gets past the feature switch is audited (claimed address, User-Agent, why it
//! was refused). The refusals are capped per minute so they cannot flood the log; a destruct that
//! fires is always written, before the destroyer has any chance to end the container.

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};

use sdw_core::audit::{event, outcome, AuditEvent};

use crate::audit::Metrics;
use crate::util::{client_ip, now_ms, user_agent};
use crate::AppState;

pub async fn destruct(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    // Checked before anything else, so a disabled endpoint is indistinguishable
    // from a missing one even if a stale key hash is still on the volume.
    if !state.features.remote_destruct {
        return StatusCode::NOT_FOUND.into_response();
    }

    let ip = client_ip(&headers);
    let ua = user_agent(&headers);
    let now = now_ms();
    let audit = |result: &'static str, reason: &'static str| {
        AuditEvent::new(now, event::DESTRUCT, result)
            .with("ip", &ip)
            .with_opt("ua", ua.as_deref())
            .with("reason", reason)
    };
    let refuse = |reason: &'static str| {
        Metrics::inc(&state.metrics.destruct_rejected);
        state.auditor.record_noisy(audit(outcome::REJECTED, reason));
        StatusCode::NOT_FOUND.into_response()
    };

    if state.limiter.lock().unwrap().check(&ip, now) {
        Metrics::inc(&state.metrics.destruct_rate_limited);
        state
            .auditor
            .record_noisy(audit(outcome::THROTTLED, "rate_limited"));
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }

    // The key hash exists only when the vault was unlocked with the feature on.
    if !std::path::Path::new(&*state.key_hash_path).exists() {
        return refuse("no_key");
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
        return refuse("bad_request");
    };

    let Ok(hash) = std::fs::read_to_string(&*state.key_hash_path) else {
        return refuse("no_key");
    };
    let hash = hash.trim().to_string();

    // bcrypt is deliberately slow; keep it off the runtime thread.
    let valid = tokio::task::spawn_blocking(move || bcrypt::verify(&passphrase, &hash))
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or(false);
    if !valid {
        return refuse("wrong_passphrase");
    }

    // Launch the destroyer and only answer success if it actually started. A failure to even
    // spawn it must not be hidden behind a 200. The child does the slow, irreversible work
    // (quiesce, unmount, shred, then SIGTERM pid 1) — far longer than the audit line needs to
    // reach the disk (quiesce stops only the vault-sync cron, not Caddy or this daemon).
    match state.destruct.trigger() {
        Ok(()) => {
            Metrics::inc(&state.metrics.destruct_triggered);
            state
                .auditor
                .record(audit(outcome::TRIGGERED, "right_passphrase"));
            (StatusCode::OK, Json(json!({ "status": "destroyed" }))).into_response()
        }
        Err(e) => {
            eprintln!("remote destruct: {e}");
            Metrics::inc(&state.metrics.destruct_error);
            state.auditor.record(audit(outcome::ERROR, "launch_failed"));
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "status": "error" })),
            )
                .into_response()
        }
    }
}
