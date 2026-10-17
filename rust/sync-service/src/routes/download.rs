//! GET /sync/api/download — port of packages/server/src/routes/download.ts.
//! No `path` query → list SHARED_DIR; with `path` → PNG-camouflaged file bytes.

use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use std::fs;
use std::path::Path;

use crate::constants::{PNG_HEADER, SHARED_DIR};
use crate::path_safety::{check_relative_path, PathCheck};
use crate::AppState;

#[derive(Deserialize)]
pub struct DownloadQuery {
    path: Option<String>,
}

pub async fn download(
    State(_state): State<AppState>,
    Query(q): Query<DownloadQuery>,
) -> axum::response::Response {
    match q.path {
        None => {
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
        Some(p) => {
            let rel = match check_relative_path(&p) {
                PathCheck::Ok(n) => n,
                PathCheck::Err(_) => {
                    return (StatusCode::NOT_FOUND, Json(json!({ "error": "not-found" }))).into_response()
                }
            };
            let full = Path::new(SHARED_DIR).join(&rel);
            match fs::read(&full) {
                Ok(data) => {
                    let mut body = Vec::with_capacity(PNG_HEADER.len() + data.len());
                    body.extend_from_slice(&PNG_HEADER);
                    body.extend_from_slice(&data);
                    ([(header::CONTENT_TYPE, "image/png")], body).into_response()
                }
                Err(_) => {
                    (StatusCode::NOT_FOUND, Json(json!({ "error": "not-found" }))).into_response()
                }
            }
        }
    }
}
