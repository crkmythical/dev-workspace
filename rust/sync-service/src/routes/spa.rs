//! Static SPA serving — port of the two static routes in
//! packages/server/src/app.ts. Caddy proxies all of /sync/* to this service, so
//! it serves the compiled sync page here as well as the API.
//!
//! The daemon reads files the image build already produced; it never executes
//! anything from them. A later change can move this to Caddy's file_server and
//! make the daemon API-only (it needs its own smoke pass; see docs/rust-migration.md).

use axum::{
    extract::{Path, State},
    http::{header::CONTENT_TYPE, StatusCode},
    response::{Html, IntoResponse, Response},
};

use crate::AppState;

pub async fn spa_index(State(state): State<AppState>) -> Response {
    match std::fs::read_to_string(format!("{}/index.html", state.spa_dir)) {
        Ok(html) => Html(html).into_response(),
        Err(_) => Html("<h1>Sync Page</h1><p>Build with: bun run build:spa</p>".to_string())
            .into_response(),
    }
}

pub async fn spa_asset(State(state): State<AppState>, Path(path): Path<String>) -> Response {
    // The captured tail is untrusted; refuse anything that could climb out.
    if path.contains("..") || path.contains('\\') {
        return StatusCode::NOT_FOUND.into_response();
    }
    match std::fs::read(format!("{}/assets/{}", state.spa_dir, path)) {
        Ok(bytes) => ([(CONTENT_TYPE, content_type_for(&path))], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Enough of a map for a Vite build: ES modules need a JS content type or the
/// browser refuses them.
fn content_type_for(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext {
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "html" => "text/html; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        _ => "application/octet-stream",
    }
}
