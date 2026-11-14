//! POST /sync/api/upload: authenticated Rust sync protocol receive path.
//!
//! Order IS the security argument and is preserved exactly:
//!   feature gate -> key available -> framing -> timestamp -> authenticate ->
//!   claim the replay slot -> act.
//! Nothing touches storage before the envelope authenticates, and the replay slot
//! is claimed only after it does. Filesystem work runs on a blocking thread.

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};

use sdw_core::sync::aad::build_aad;
use sdw_core::sync::crypto::decrypt;
use sdw_core::sync::path_safety::{check_relative_path, PathCheck};
use sdw_core::sync::protocol::{
    HEADER_CHUNK_IDX, HEADER_FILE_ID, HEADER_OP, HEADER_TS, MAX_UPLOAD_BYTES, MAX_UPLOAD_CHUNKS,
    REPLAY_TOLERANCE_MS,
};
use sdw_core::sync::wire::parse_upload_body;

use crate::store;
use crate::util::now_ms;
use crate::AppState;

fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

/// One safe path segment (a file id names a temp file).
fn flat_segment(input: &str) -> Option<String> {
    match check_relative_path(input) {
        PathCheck::Ok(n) if !n.contains('/') => Some(n),
        _ => None,
    }
}

pub async fn upload(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    // Off unless SYNC_ENABLED: a disabled feature looks like a missing one.
    if !state.features.sync {
        return StatusCode::NOT_FOUND.into_response();
    }

    let key = match state.sync_key() {
        Some(k) => k,
        None => return err(StatusCode::SERVICE_UNAVAILABLE, "vault-locked"),
    };

    let frame = match parse_upload_body(body.as_ref()) {
        Some(f) => f,
        None => return err(StatusCode::BAD_REQUEST, "too-short"),
    };

    let hdr = |name: &str, default: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or(default)
            .to_string()
    };
    let op = hdr(HEADER_OP, "put-chunk");
    let now = now_ms();
    // A missing timestamp falls back to "now" (the AAD then will not match and the
    // envelope fails to authenticate, exactly as before); a present-but-unparseable
    // one is a bad timestamp.
    let ts: i64 = match headers.get(HEADER_TS).and_then(|v| v.to_str().ok()) {
        None => now,
        Some(s) => match s.parse::<i64>() {
            Ok(n) => n,
            Err(_) => return err(StatusCode::BAD_REQUEST, "bad-timestamp"),
        },
    };
    let file_id_hdr = hdr(HEADER_FILE_ID, "");
    let chunk_idx = hdr(HEADER_CHUNK_IDX, "0");

    let aad = build_aad(&op, ts, &[file_id_hdr.as_str(), chunk_idx.as_str()]);
    let plaintext = match decrypt(&key, frame.nonce, frame.ct_with_tag, &aad) {
        Ok(p) => p,
        Err(_) => return err(StatusCode::BAD_REQUEST, "aead-failure"),
    };

    // A present-but-out-of-tolerance timestamp is a stale/clock problem, distinct from a replay.
    // Report it as such instead of the misleading "replay-detected" the window would otherwise
    // return for the same condition.
    if now.abs_diff(ts) > REPLAY_TOLERANCE_MS as u64 {
        return err(StatusCode::BAD_REQUEST, "stale-timestamp");
    }

    // Only an authentic envelope may claim a replay-window slot.
    {
        let nonce_hex = hex::encode(frame.nonce);
        let mut rw = state.replay.lock().unwrap();
        if !rw.check(&nonce_hex, ts, now) {
            return err(StatusCode::BAD_REQUEST, "replay-detected");
        }
    }

    let dirs = state.dirs.clone();

    if op == "put-chunk" {
        let Some(id) = flat_segment(&file_id_hdr) else {
            return err(StatusCode::BAD_REQUEST, "bad-file-id");
        };
        let index = match chunk_idx.parse::<u64>() {
            Ok(index) if index < MAX_UPLOAD_CHUNKS => index,
            _ => return err(StatusCode::BAD_REQUEST, "bad-chunk-index"),
        };
        let Some(session) = state.upload_session(&id, now).await else {
            return err(StatusCode::SERVICE_UNAVAILABLE, "too-many-uploads");
        };
        let mut session = session.lock().await;
        session.last_seen_ms = now;
        if session.finalized {
            return err(StatusCode::CONFLICT, "upload-finalized");
        }
        if index != session.next_chunk {
            return err(StatusCode::CONFLICT, "out-of-order-chunk");
        }
        let res = tokio::task::spawn_blocking(move || {
            store::append_chunk(&dirs, &id, &plaintext, MAX_UPLOAD_BYTES)
        })
        .await;
        match res {
            Ok(Ok(())) => session.next_chunk += 1,
            Ok(Err(store::AppendError::TooLarge)) => {
                return err(StatusCode::PAYLOAD_TOO_LARGE, "upload-too-large")
            }
            _ => return err(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        }
        return (
            StatusCode::OK,
            Json(json!({ "status": "chunk-received", "chunk_idx": index })),
        )
            .into_response();
    }

    if op == "put-finalize" {
        let data = match serde_json::from_slice::<Value>(&plaintext) {
            Ok(v) if v.is_object() => v,
            _ => return err(StatusCode::BAD_REQUEST, "bad-finalize-json"),
        };
        let Some(id) = flat_segment(data.get("file_id").and_then(Value::as_str).unwrap_or(""))
        else {
            return err(StatusCode::BAD_REQUEST, "bad-file-id");
        };
        if id != file_id_hdr {
            return err(StatusCode::BAD_REQUEST, "file-id-mismatch");
        }
        let total_chunks = match data.get("total_chunks").and_then(Value::as_u64) {
            Some(total) if (1..=MAX_UPLOAD_CHUNKS).contains(&total) => total,
            _ => return err(StatusCode::BAD_REQUEST, "bad-total-chunks"),
        };
        if chunk_idx.parse::<u64>().ok() != Some(total_chunks) {
            return err(StatusCode::BAD_REQUEST, "chunk-count-mismatch");
        }
        let rel = match check_relative_path(
            data.get("file_path").and_then(Value::as_str).unwrap_or(""),
        ) {
            PathCheck::Ok(n) => n,
            PathCheck::Err(_) => return err(StatusCode::BAD_REQUEST, "bad-file-path"),
        };

        // Whole-file integrity and the declared chunk count are both required. The per-id session
        // below serializes finalize with append, so no chunk can slip between hashing and rename.
        let expected = match data.get("content_sha256").and_then(Value::as_str) {
            Some(h)
                if h.len() == 64
                    && h.bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()) =>
            {
                h.to_string()
            }
            _ => return err(StatusCode::BAD_REQUEST, "missing-checksum"),
        };

        let Some(upload) = state.upload_session(&id, now).await else {
            return err(StatusCode::SERVICE_UNAVAILABLE, "too-many-uploads");
        };
        let mut upload = upload.lock().await;
        upload.last_seen_ms = now;
        if upload.finalized {
            return err(StatusCode::CONFLICT, "upload-finalized");
        }
        if upload.next_chunk != total_chunks {
            return err(StatusCode::CONFLICT, "chunk-count-mismatch");
        }

        let outcome = {
            let rel = rel.clone();
            tokio::task::spawn_blocking(move || {
                store::finalize_verified(&dirs, &id, &rel, &expected)
            })
            .await
        };
        return match outcome {
            Ok(Ok(store::FinalizeOutcome::Committed)) => {
                upload.finalized = true;
                (
                    StatusCode::OK,
                    Json(json!({ "status": "upload-complete", "path": rel })),
                )
                    .into_response()
            }
            Ok(Ok(store::FinalizeOutcome::ChecksumMismatch)) => {
                upload.next_chunk = 0;
                err(StatusCode::BAD_REQUEST, "checksum-mismatch")
            }
            Ok(Ok(store::FinalizeOutcome::NoChunks)) => {
                upload.next_chunk = 0;
                err(StatusCode::BAD_REQUEST, "no-chunks")
            }
            _ => err(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        };
    }

    err(StatusCode::BAD_REQUEST, "unknown-op")
}
