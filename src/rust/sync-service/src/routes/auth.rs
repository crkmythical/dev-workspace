//! The unified auth gateway's HTTP surface (served by the always-on sync-service, allowlisted
//! un-gated in Caddy):
//!   GET  /auth/login   — login form (reflects a safe `next`)
//!   POST /auth/login   — verify the master password, set the session cookie, redirect
//!   GET  /auth/verify  — the Caddy `forward_auth` target: 200 if the cookie is valid, else 401
//!   POST /auth/logout  — clear the cookie
//!
//! The session cookie rides same-origin WebSocket handshakes, so one gate covers code-server,
//! KasmVNC and Selkies WS alike. See deploy/docs/design-unified-auth-gateway.md.

use axum::{
    extract::{Form, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
};
use serde::Deserialize;

use sdw_core::auth::session::{sign_session, verify_session, SessionVerdict, SESSION_COOKIE};

use crate::util::now_ms;
use crate::AppState;

#[derive(Deserialize)]
pub struct LoginQuery {
    pub next: Option<String>,
}

#[derive(Deserialize)]
pub struct LoginForm {
    pub password: String,
    pub next: Option<String>,
}

fn now_secs() -> u64 {
    now_ms().max(0) as u64 / 1000
}

// No `Secure`: local access is plain HTTP on 127.0.0.1 and the remote leg is the encrypted tunnel,
// so requiring HTTPS here would lock out localhost. Loopback has no MITM to protect against.
const COOKIE_ATTRS: &str = "HttpOnly; SameSite=Lax; Path=/";

fn set_cookie(token: &str, max_age: u64) -> String {
    format!("{SESSION_COOKIE}={token}; {COOKIE_ATTRS}; Max-Age={max_age}")
}

fn clear_cookie() -> String {
    format!("{SESSION_COOKIE}=; {COOKIE_ATTRS}; Max-Age=0")
}

/// The value of a named cookie from the request `Cookie` header, if present.
fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k.trim() == name).then_some(v.trim())
    })
}

/// A redirect target we will honour after login: local, absolute, not protocol-relative (so it
/// cannot be turned into an open redirect to another site).
fn safe_next(next: Option<&str>) -> String {
    match next {
        Some(n) if n.starts_with('/') && !n.starts_with("//") => n.to_string(),
        _ => "/".to_string(),
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn login_page(next_escaped: &str, error: bool) -> Html<String> {
    let msg = if error {
        r#"<p class="e">Wrong password.</p>"#
    } else {
        ""
    };
    Html(format!(
        "<!doctype html><meta charset=utf-8>\
<meta name=viewport content=\"width=device-width,initial-scale=1\">\
<title>Unlock workspace</title>\
<style>body{{font-family:system-ui,sans-serif;background:#111;color:#eee;\
display:grid;place-items:center;height:100vh;margin:0}}\
form{{background:#1b1b1b;padding:2rem;border-radius:10px;min-width:260px}}\
h3{{margin:0 0 1rem}}input,button{{width:100%;box-sizing:border-box;padding:.6rem;\
margin:.3rem 0;border-radius:6px;border:1px solid #333;background:#222;color:#eee}}\
button{{background:#2d6cdf;border:0;cursor:pointer}}.e{{color:#ff6b6b;margin:0 0 .5rem}}</style>\
<form method=post action=\"/auth/login\"><h3>Secure Dev Workspace</h3>{msg}\
<input type=hidden name=next value=\"{next_escaped}\">\
<input type=password name=password placeholder=\"Master password\" autofocus \
autocomplete=current-password>\
<button type=submit>Unlock</button></form>"
    ))
}

pub async fn login_get(Query(q): Query<LoginQuery>) -> Html<String> {
    login_page(&html_escape(&safe_next(q.next.as_deref())), false)
}

pub async fn login_post(State(st): State<AppState>, Form(f): Form<LoginForm>) -> Response {
    let next = safe_next(f.next.as_deref());
    let ok = match &st.auth.password_hash {
        Some(h) => bcrypt::verify(&f.password, h.as_ref()).unwrap_or(false),
        None => false,
    };
    if !ok {
        return (
            StatusCode::UNAUTHORIZED,
            login_page(&html_escape(&next), true),
        )
            .into_response();
    }
    let now = now_secs();
    let token = sign_session(&st.auth.session_key[..], now, now + st.auth.ttl_secs);
    (
        StatusCode::SEE_OTHER,
        [
            (header::SET_COOKIE, set_cookie(&token, st.auth.ttl_secs)),
            (header::LOCATION, next),
        ],
    )
        .into_response()
}

/// The `forward_auth` target. Caddy copies the client's headers here; we only need the cookie.
pub async fn verify(State(st): State<AppState>, headers: HeaderMap) -> StatusCode {
    match cookie_value(&headers, SESSION_COOKIE) {
        Some(tok)
            if verify_session(&st.auth.session_key[..], tok, now_secs())
                == SessionVerdict::Valid =>
        {
            StatusCode::OK
        }
        _ => StatusCode::UNAUTHORIZED,
    }
}

pub async fn logout() -> Response {
    (
        StatusCode::SEE_OTHER,
        [
            (header::SET_COOKIE, clear_cookie()),
            (header::LOCATION, "/auth/login".to_string()),
        ],
    )
        .into_response()
}
