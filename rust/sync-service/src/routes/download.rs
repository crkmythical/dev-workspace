//! GET /sync/api/download — port of packages/server/src/routes/download.ts.
//!
//! Lists SHARED_DIR, and only that. File content is deliberately not served (the
//! SPA never downloads it, and the old `?path=` branch returned plaintext with no
//! authentication). Listing demands the same proof of the sync key as an upload,
//! see `list_auth`.
use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::constants::SHARED_DIR;
use crate::list_auth::authenticate_list;
use crate::AppState;

#[derive(Deserialize)]
pub struct DownloadQuery {
    path: Option<String>,
}

fn err(status: StatusCode, msg: &str) -> axum::response::Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

pub async fn download(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<DownloadQuery>,
) -> axum::response::Response {
    // Content download is gone: say so, rather than quietly ignoring the parameter.
    if q.path.is_some() {
        return StatusCode::NOT_FOUND.into_response();
    }

    let key = match state.sync_key() {
        Some(k) => k,
        None => return err(StatusCode::SERVICE_UNAVAILABLE, "vault-locked"),
    };

    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let ts = header("x-sync-ts").and_then(|v| v.parse::<i64>().ok());
    let (Some(ts), Some(auth)) = (ts, header("x-sync-auth")) else {
        return err(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    if authenticate_list(&key, ts, auth, &state.replay, now_ms()).is_err() {
        return err(StatusCode::UNAUTHORIZED, "unauthorized");
    }

    let mut files = Vec::new();
    if let Ok(rd) = fs::read_dir(SHARED_DIR) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            if let Ok(md) = entry.metadata() {
                if md.is_file() {
                    files.push(json!({ "name": name, "size": md.len() }));
                }
            }
        }
    }
    Json(json!({ "files": files })).into_response()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
