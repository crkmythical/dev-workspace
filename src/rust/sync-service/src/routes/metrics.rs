//! GET /sync/api/metrics — Prometheus text counters (logins, gate decisions, self-destruct
//! requests). Off unless METRICS_ENABLED, and then a 404 like any other disabled feature. Like all
//! of /sync/api/* it sits behind the ingress gate, so a scraper needs a session cookie or a
//! Cloudflare Access service token; from inside the container, `curl 127.0.0.1:8081/sync/api/metrics`.

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};

use crate::AppState;

pub async fn metrics(State(state): State<AppState>) -> Response {
    if !state.metrics_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        state.metrics.render(),
    )
        .into_response()
}
