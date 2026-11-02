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
    use sdw_core::auth::session::{AUTH_LOGIN_PATH, AUTH_LOGOUT_PATH, AUTH_VERIFY_PATH};
    use sdw_core::sync::protocol::{
        API_DESTRUCT, API_DOCTOR, API_DOWNLOAD, API_UPLOAD, MAX_REQUEST_BODY_BYTES,
    };
    Router::new()
        .route(API_UPLOAD, post(routes::upload::upload))
        .route(API_DOWNLOAD, get(routes::download::download))
        .route(API_DOCTOR, get(routes::doctor::doctor))
        .route(API_DESTRUCT, post(routes::destruct::destruct))
        // Unified auth gateway (allowlisted un-gated in Caddy; the rest sits behind /auth/verify).
        .route(
            AUTH_LOGIN_PATH,
            get(routes::auth::login_get).post(routes::auth::login_post),
        )
        .route(AUTH_VERIFY_PATH, get(routes::auth::verify))
        .route(AUTH_LOGOUT_PATH, post(routes::auth::logout))
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
        // When behind Cloudflare Access, prime and keep the team JWKS fresh so /auth/verify can
        // accept Access assertions. The initial fetch is awaited so the keys are ready before the
        // first request; a background task then refreshes them.
        if let Some(access) = state.auth.access.clone() {
            spawn_jwks_refresh(access);
        }
        let addr = format!("127.0.0.1:{}", sdw_core::sync::layout::SYNC_SERVICE_PORT);
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .expect("bind sync-service port");
        println!("sync-service (rust) listening on {addr}");
        axum::serve(listener, app(state)).await.expect("serve");
    });
}

/// Keep the Access JWKS fresh. Fetch immediately, then retry QUICKLY until the first success — at
/// boot the egress proxy (Clash) the fetch goes through may still be coming up, so the initial try
/// can fail, and waiting an hour to retry would leave Access single-sign-on broken until then. Once
/// keys are in hand, refresh hourly for rotation. Errors are swallowed: the cookie path is
/// unaffected, and an empty JWKS just means an Access assertion is not accepted until keys arrive.
fn spawn_jwks_refresh(access: state::AccessState) {
    tokio::spawn(async move {
        loop {
            let have_keys = match fetch_jwks(&access.certs_url).await {
                Some(jwks) if !jwks.keys.is_empty() => {
                    if let Ok(mut w) = access.jwks.write() {
                        *w = jwks;
                    }
                    true
                }
                _ => false,
            };
            let delay = if have_keys { 3600 } else { 20 };
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
        }
    });
}

/// Fetch the team JWKS with curl (so it uses the same proxy/TLS the rest of the image does, and no
/// HTTP/TLS client is linked into the binary). None on any failure.
async fn fetch_jwks(certs_url: &str) -> Option<sdw_core::auth::access_jwt::Jwks> {
    let out = tokio::process::Command::new("curl")
        .args(["-fsS", "--max-time", "15", certs_url])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}
