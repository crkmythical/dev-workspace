//! GET /sync/api/download — port of packages/server/src/routes/download.ts.
//!
//! Lists the shared directory, and only that. File content is deliberately not
//! served (the SPA never downloads it, and the old `?path=` branch returned
//! plaintext with no authentication). Listing demands the same proof of the sync
//! key as an upload.

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;

use crate::list_auth::authenticate_list;
use crate::store;
use crate::util::now_ms;
use crate::AppState;

#[derive(Deserialize)]
pub struct DownloadQuery {
    path: Option<String>,
}

fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

pub async fn download(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<DownloadQuery>,
) -> Response {
    // Off unless SYNC_ENABLED: a disabled feature looks like a missing one.
    if !state.features.sync {
        return StatusCode::NOT_FOUND.into_response();
    }
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

    let dirs = state.dirs.clone();
    let files = tokio::task::spawn_blocking(move || store::list(&dirs))
        .await
        .unwrap_or_default();
    Json(json!({ "files": files })).into_response()
}
