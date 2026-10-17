//! POST /sync/api/upload — faithful port of packages/server/src/routes/upload.ts.
//! Uses std::fs synchronously (PoC): the handler has no `.await`, so no lock is
//! ever held across a suspension point.

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::aad::build_aad;
use crate::constants::{SHARED_DIR, UPLOAD_TRACKING_DIR};
use crate::crypto::decrypt;
use crate::path_safety::{check_relative_path, PathCheck};
use crate::wire::parse_upload_body;
use crate::AppState;

fn err(status: StatusCode, msg: &str) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "error": msg })))
}

pub async fn upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, Json<Value>) {
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
    let ts: i64 = hdr("x-sync-ts", "0").parse().unwrap_or(0);
    let file_id_hdr = hdr("x-sync-fileid", "");
    let chunk_idx = hdr("x-sync-chunkidx", "0");

    let aad = build_aad(&op, ts, &[file_id_hdr.as_str(), chunk_idx.as_str()]);
    let plaintext = match decrypt(&key, frame.nonce, frame.ct_with_tag, &aad) {
        Ok(p) => p,
        Err(_) => return err(StatusCode::BAD_REQUEST, "aead-failure"),
    };

    // Replay protection. Only an authentic envelope may claim a window slot: the
    // window is bounded, so forgeries that could claim slots would flush genuine
    // nonces out and let a captured request be replayed.
    {
        let nonce_hex = hex::encode(frame.nonce);
        let mut rw = state.replay.lock().unwrap();
        if !rw.check(&nonce_hex, ts, now_ms()) {
            return err(StatusCode::BAD_REQUEST, "replay-detected");
        }
    }

    let _ = fs::create_dir_all(UPLOAD_TRACKING_DIR);
    let _ = fs::create_dir_all(SHARED_DIR);

    if op == "put-chunk" {
        let id = match check_relative_path(&file_id_hdr) {
            PathCheck::Ok(n) if !n.contains('/') => n,
            _ => return err(StatusCode::BAD_REQUEST, "bad-file-id"),
        };
        let temp = Path::new(UPLOAD_TRACKING_DIR).join(format!(".uploading-{id}"));
        if append_file(&temp, &plaintext).is_err() {
            return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
        return (
            StatusCode::OK,
            Json(json!({ "status": "chunk-received", "chunk_idx": chunk_idx.parse::<i64>().unwrap_or(0) })),
        );
    }

    if op == "put-finalize" {
        let data: Value = match serde_json::from_slice(&plaintext) {
            Ok(v) => v,
            Err(_) => return err(StatusCode::BAD_REQUEST, "bad-finalize-json"),
        };
        let file_id = data.get("file_id").and_then(Value::as_str).unwrap_or("");
        let file_path = data.get("file_path").and_then(Value::as_str).unwrap_or("");

        let id = match check_relative_path(file_id) {
            PathCheck::Ok(n) if !n.contains('/') => n,
            _ => return err(StatusCode::BAD_REQUEST, "bad-file-id"),
        };
        let rel = match check_relative_path(file_path) {
            PathCheck::Ok(n) => n,
            PathCheck::Err(_) => return err(StatusCode::BAD_REQUEST, "bad-file-path"),
        };

        let temp = Path::new(UPLOAD_TRACKING_DIR).join(format!(".uploading-{id}"));
        if !temp.exists() {
            return err(StatusCode::BAD_REQUEST, "no-chunks");
        }

        if let Some(expected) = data.get("content_sha256").and_then(Value::as_str) {
            let bytes = match fs::read(&temp) {
                Ok(b) => b,
                Err(_) => return err(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
            };
            let mut hsh = Sha256::new();
            hsh.update(&bytes);
            if hex::encode(hsh.finalize()) != expected {
                let _ = fs::remove_file(&temp);
                return err(StatusCode::BAD_REQUEST, "checksum-mismatch");
            }
        }

        let final_path = Path::new(SHARED_DIR).join(&rel);
        if let Some(parent) = final_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if fs::rename(&temp, &final_path).is_err() {
            return err(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        }
        return (StatusCode::OK, Json(json!({ "status": "upload-complete", "path": rel })));
    }

    err(StatusCode::BAD_REQUEST, "unknown-op")
}

fn append_file(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut f = fs::OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(data)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
