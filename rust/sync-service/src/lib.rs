//! Rust/Axum PoC of the sync-service. Wire-compatible with the TypeScript
//! implementation in packages/server + packages/core. See README.md.

pub mod aad;
pub mod constants;
pub mod crypto;
pub mod path_safety;
pub mod replay;
pub mod routes;
pub mod state;
pub mod wire;

pub use state::AppState;

use axum::{
    routing::{get, post},
    Router,
};

/// Build the application router with the given shared state.
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/sync/api/upload", post(routes::upload::upload))
        .route("/sync/api/download", get(routes::download::download))
        .route("/sync/api/doctor", get(routes::doctor::doctor))
        .with_state(state)
}
