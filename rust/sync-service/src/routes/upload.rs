//! POST /sync/api/upload — port of packages/server/src/routes/upload.ts over the
//! receiveUpload() use case (packages/core/src/sync/usecases.ts).
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

use crate::aad::build_aad;
use crate::crypto::decrypt;
use crate::path_safety::{check_relative_path, PathCheck};
use crate::store;
use crate::util::now_ms;
use crate::wire::parse_upload_body;
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
    let op = hdr("x-sync-op", "put-chunk");
    let now = now_ms();
    // A missing timestamp falls back to "now" (the AAD then will not match and the
    // envelope fails to authenticate, exactly as before); a present-but-unparseable
    // one is a bad timestamp.
    let ts: i64 = match headers.get("x-sync-ts").and_then(|v| v.to_str().ok()) {
        None => now,
        Some(s) => match s.parse::<i64>() {
            Ok(n) => n,
            Err(_) => return err(StatusCode::BAD_REQUEST, "bad-timestamp"),
        },
    };
    let file_id_hdr = hdr("x-sync-fileid", "");
    let chunk_idx = hdr("x-sync-chunkidx", "0");

    let aad = build_aad(&op, ts, &[file_id_hdr.as_str(), chunk_idx.as_str()]);
    let plaintext = match decrypt(&key, frame.nonce, frame.ct_with_tag, &aad) {
        Ok(p) => p,
        Err(_) => return err(StatusCode::BAD_REQUEST, "aead-failure"),
    };

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
        let res =
            tokio::task::spawn_blocking(move || store::append_chunk(&dirs, &id, &plaintext)).await;
        if !matches!(res, Ok(Ok(()))) {
            return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
        let idx = chunk_idx.parse::<i64>().unwrap_or(0);
        return (
            StatusCode::OK,
            Json(json!({ "status": "chunk-received", "chunk_idx": idx })),
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
        let rel = match check_relative_path(
            data.get("file_path").and_then(Value::as_str).unwrap_or(""),
        ) {
            PathCheck::Ok(n) => n,
            PathCheck::Err(_) => return err(StatusCode::BAD_REQUEST, "bad-file-path"),
        };

        {
            let dirs = dirs.clone();
            let id = id.clone();
            let present = tokio::task::spawn_blocking(move || store::has_chunks(&dirs, &id))
                .await
                .unwrap_or(false);
            if !present {
                return err(StatusCode::BAD_REQUEST, "no-chunks");
            }
        }

        if let Some(expected) = data.get("content_sha256").and_then(Value::as_str) {
            let got = {
                let dirs = dirs.clone();
                let id = id.clone();
                tokio::task::spawn_blocking(move || store::sha256_hex(&dirs, &id)).await
            };
            let got = match got {
                Ok(Ok(h)) => h,
                _ => return err(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
            };
            if got != expected {
                let dirs = dirs.clone();
                let id = id.clone();
                let _ = tokio::task::spawn_blocking(move || store::discard(&dirs, &id)).await;
                return err(StatusCode::BAD_REQUEST, "checksum-mismatch");
            }
        }

        let res = {
            let rel = rel.clone();
            tokio::task::spawn_blocking(move || store::commit(&dirs, &id, &rel)).await
        };
        if !matches!(res, Ok(Ok(()))) {
            return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
        return (
            StatusCode::OK,
            Json(json!({ "status": "upload-complete", "path": rel })),
        )
            .into_response();
    }

    err(StatusCode::BAD_REQUEST, "unknown-op")
}
