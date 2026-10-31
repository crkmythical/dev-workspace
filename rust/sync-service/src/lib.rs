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
    routing::{get, post},
    Router,
};

/// Build the application router. The full /sync/* surface Caddy proxies here: the
/// static sync page plus the upload / download / doctor / destruct API.
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/sync/", get(routes::spa::spa_index))
        .route("/sync/assets/*path", get(routes::spa::spa_asset))
        .route("/sync/api/upload", post(routes::upload::upload))
        .route("/sync/api/download", get(routes::download::download))
        .route("/sync/api/doctor", get(routes::doctor::doctor))
        .route("/sync/api/destruct", post(routes::destruct::destruct))
        .with_state(state)
}
