use sync_service::{app, constants::SYNC_SERVICE_PORT, AppState};

#[tokio::main]
async fn main() {
    let state = AppState::new();
    let router = app(state);
    let addr = format!("127.0.0.1:{SYNC_SERVICE_PORT}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind sync-service port");
    println!("sync-service (rust) listening on {addr}");
    axum::serve(listener, router).await.expect("serve");
}
