//! Rust/Axum sync-service. Wire-compatible with the TypeScript implementation it
//! replaces (packages/server + packages/core), proven by tests/parity.rs against
//! golden vectors from the authoritative TS side, and by the shared HTTP contract
//! in tests/smoke. See README.md.

pub mod destruct;
pub mod features;
pub mod list_auth;
pub mod routes;
pub mod state;
pub mod store;
pub mod util;

pub use state::{AppConfig, AppState};

use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post},
    Router,
};

/// Build the application router: the /sync/api/* surface (upload / download / doctor /
/// destruct). The static SPA under /sync/ is served by Caddy's file_server, not here.
pub fn app(state: AppState) -> Router {
    use sdw_core::sync::protocol::{
        API_DESTRUCT, API_DOCTOR, API_DOWNLOAD, API_UPLOAD, MAX_REQUEST_BODY_BYTES,
    };
    Router::new()
        .route(API_UPLOAD, post(routes::upload::upload))
        .route(API_DOWNLOAD, get(routes::download::download))
        .route(API_DOCTOR, get(routes::doctor::doctor))
        .route(API_DESTRUCT, post(routes::destruct::destruct))
        // axum's default `Bytes` limit is 2 MiB, but the browser uploads up to 4 MiB chunks;
        // without this a real chunk is rejected 413 before the handler and files over ~2 MB
        // cannot be uploaded at all.
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
        .with_state(state)
}

/// Build a Tokio runtime and serve the API until the process is terminated. This is the body of
/// the sync daemon; the `sdw sync-service` subcommand calls it, so the image ships one Rust binary
/// (`sdw`) instead of a separate sync-service executable.
pub fn run_blocking() {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let state = AppState::from_config(AppConfig::from_env());
        let addr = format!("127.0.0.1:{}", sdw_core::sync::layout::SYNC_SERVICE_PORT);
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .expect("bind sync-service port");
        println!("sync-service (rust) listening on {addr}");
        axum::serve(listener, app(state)).await.expect("serve");
    });
}
