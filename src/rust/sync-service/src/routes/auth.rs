//! The unified auth gateway's HTTP surface (served by the always-on sync-service, allowlisted
//! un-gated in Caddy):
//!   GET  /auth/login   — login form (reflects a safe `next`)
//!   POST /auth/login   — verify the master password, set the session cookie, redirect
//!   GET  /auth/verify  — the Caddy `forward_auth` target: 200 if the cookie is valid, else 401
//!   POST /auth/logout  — clear the cookie
//!
//! The session cookie rides same-origin WebSocket handshakes, so one gate covers code-server,
//! KasmVNC and Selkies WS alike. See deploy/docs/design-unified-auth-gateway.md.
//!
//! Logins are throttled (every attempt counts) and audited: who (claimed address, User-Agent),
//! when, and the result. The password itself never reaches the log.

use axum::{
    extract::{Form, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
};
use serde::Deserialize;

use sdw_core::audit::{event, outcome, AuditEvent};
use sdw_core::auth::access_jwt::{verify_access_jwt_identity, AccessError, AccessIdentity};
use sdw_core::auth::request::{arrived_over_https, is_cross_origin_websocket};
use sdw_core::auth::session::{sign_session, verify_session, SessionVerdict, SESSION_COOKIE};

use crate::audit::Metrics;
use crate::state::AccessState;
use crate::util::{client_ip, now_ms, user_agent};
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

const COOKIE_ATTRS: &str = "HttpOnly; SameSite=Lax; Path=/";

/// A request header as text, if it is there and is text.
fn header_text<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// `Secure` only when the client reached us over HTTPS (Cloudflare's CF-Visitor says so, see
/// core::auth::request). Local access is plain HTTP on 127.0.0.1, where a `Secure` cookie would not
/// be kept by Safari.
fn secure_flag(headers: &HeaderMap) -> &'static str {
    if arrived_over_https(
        header_text(headers, "x-forwarded-proto"),
        header_text(headers, "cf-visitor"),
    ) {
        "; Secure"
    } else {
        ""
    }
}

fn set_cookie(token: &str, max_age: u64, secure: &str) -> String {
    format!("{SESSION_COOKIE}={token}; {COOKIE_ATTRS}{secure}; Max-Age={max_age}")
}

fn clear_cookie(secure: &str) -> String {
    format!("{SESSION_COOKIE}=; {COOKIE_ATTRS}{secure}; Max-Age=0")
}

/// The value of a named cookie from the request `Cookie` header, if present.
fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k.trim() == name).then_some(v.trim())
    })
}

/// A redirect target we will honour after login: local, absolute, and not something a browser can
/// read as another site. Not `//host` (protocol-relative), and not `/\host`: browsers treat a
/// backslash as a slash, and strip tabs and newlines out of a URL, so `/<TAB>/host` is `//host`.
fn safe_next(next: Option<&str>) -> String {
    match next {
        Some(n)
            if n.starts_with('/')
                && !n.starts_with("//")
                && !n.chars().any(|c| c.is_control() || c == '\\') =>
        {
            n.to_string()
        }
        _ => "/".to_string(),
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// What the login page says above the form.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Notice {
    None,
    WrongPassword,
    TooManyAttempts,
}

fn login_page(next_escaped: &str, notice: Notice) -> Html<String> {
    let msg = match notice {
        Notice::None => "",
        Notice::WrongPassword => r#"<p class="e">Wrong password.</p>"#,
        Notice::TooManyAttempts => r#"<p class="e">Too many attempts. Try again in a minute.</p>"#,
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

/// Headers for the login page: not framed (clickjacking), not cached, no scripts or other
/// resources, and nothing leaked through the Referer.
fn page_headers() -> [(header::HeaderName, &'static str); 5] {
    [
        (header::CACHE_CONTROL, "no-store"),
        (header::X_FRAME_OPTIONS, "DENY"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::REFERRER_POLICY, "no-referrer"),
        (
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; \
             frame-ancestors 'none'; base-uri 'none'",
        ),
    ]
}

pub async fn login_get(Query(q): Query<LoginQuery>) -> Response {
    (
        page_headers(),
        login_page(&html_escape(&safe_next(q.next.as_deref())), Notice::None),
    )
        .into_response()
}

pub async fn login_post(
    State(st): State<AppState>,
    headers: HeaderMap,
    Form(f): Form<LoginForm>,
) -> Response {
    let next = safe_next(f.next.as_deref());
    let ip = client_ip(&headers);
    let ua = user_agent(&headers);
    let now = now_ms();
    let audit = |result: &'static str| {
        AuditEvent::new(now, event::LOGIN, result)
            .with("ip", &ip)
            .with_opt("ua", ua.as_deref())
    };

    // Throttle before any bcrypt work: the master password is also the vault passphrase, so this
    // endpoint must not be an unlimited guessing oracle, and the slow hash must not be a way to
    // keep the runtime busy. Every attempt counts, right or wrong.
    let throttled = st
        .login_limiter
        .lock()
        .map(|mut l| l.check(&ip, now))
        .unwrap_or(false);
    if throttled {
        Metrics::inc(&st.metrics.login_throttled);
        st.auditor.record_noisy(audit(outcome::THROTTLED));
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, "60")],
            page_headers(),
            login_page(&html_escape(&next), Notice::TooManyAttempts),
        )
            .into_response();
    }

    // bcrypt is deliberately slow: run it on the blocking pool, not on a runtime worker (the gate
    // shares this runtime and Caddy asks it about every request).
    let hash = st.auth.password_hash.clone();
    let password = f.password;
    let ok = tokio::task::spawn_blocking(move || match hash {
        Some(h) => bcrypt::verify(&password, h.as_ref()).unwrap_or(false),
        None => false,
    })
    .await
    .unwrap_or(false);

    if !ok {
        Metrics::inc(&st.metrics.login_failure);
        st.auditor.record_noisy(audit(outcome::FAILURE));
        return (
            StatusCode::UNAUTHORIZED,
            page_headers(),
            login_page(&html_escape(&next), Notice::WrongPassword),
        )
            .into_response();
    }
    Metrics::inc(&st.metrics.login_success);
    st.auditor.record(audit(outcome::SUCCESS));
    let now_s = now_secs();
    let token = sign_session(&st.auth.session_key[..], now_s, now_s + st.auth.ttl_secs);
    (
        StatusCode::SEE_OTHER,
        [
            (
                header::SET_COOKIE,
                set_cookie(&token, st.auth.ttl_secs, secure_flag(&headers)),
            ),
            (header::LOCATION, next),
        ],
    )
        .into_response()
}

/// The `forward_auth` target. Caddy copies the client's headers here. Two accepted credentials:
/// (1) our own session cookie, (2) a valid Cloudflare Access assertion when the deployment is
/// behind Access (so an Access login is not asked to log in again). Either passing → 200.
///
/// Neither is accepted for a WebSocket handshake from a page on another origin (403): the cookie
/// is SameSite=Lax, which is "same site", not "same origin", so a page on a sibling subdomain would
/// otherwise open a desktop or terminal socket with the visitor's session.
///
/// Called for every gated request, so a cookie pass is only counted (metrics), never logged. An
/// Access sign-in is logged once per identity per quarter hour. Denials are counted, not logged:
/// any anonymous request produces one.
pub async fn verify(State(st): State<AppState>, headers: HeaderMap) -> StatusCode {
    let now = now_secs();
    // 0) The origin rule comes first: it applies whatever credential the request carries.
    if is_cross_origin_websocket(
        header_text(&headers, "upgrade"),
        header_text(&headers, "origin"),
        header_text(&headers, "host"),
        header_text(&headers, "x-forwarded-host"),
    ) {
        Metrics::inc(&st.metrics.gate_deny_origin);
        st.auditor.record_noisy(
            AuditEvent::new(now_ms(), event::GATE_ORIGIN, outcome::REJECTED)
                .with("ip", client_ip(&headers))
                .with_opt("origin", header_text(&headers, "origin"))
                .with_opt("host", header_text(&headers, "host")),
        );
        return StatusCode::FORBIDDEN;
    }
    // 1) Our session cookie.
    if let Some(tok) = cookie_value(&headers, SESSION_COOKIE) {
        if verify_session(&st.auth.session_key[..], tok, now) == SessionVerdict::Valid {
            Metrics::inc(&st.metrics.gate_allow_cookie);
            return StatusCode::OK;
        }
    }
    // 2) Cloudflare Access assertion, only if configured (else the header is never trusted).
    if let Some(access) = &st.auth.access {
        if let Some(jwt) = header_text(&headers, "cf-access-jwt-assertion") {
            let mut checked = check_access(access, jwt, now);
            // A key id we do not have is how a rotation of Cloudflare's signing keys looks from
            // here. Fetch the keys once (rate-limited) and look again, instead of refusing every
            // Access user until the hourly refresh.
            if checked == Err(AccessError::UnknownKid)
                && access.claim_refetch(now_ms())
                && crate::refresh_jwks(access).await
            {
                checked = check_access(access, jwt, now);
            }
            if let Ok(who) = checked {
                Metrics::inc(&st.metrics.gate_allow_access);
                st.auditor
                    .record_access(now_ms(), &who.label(), &client_ip(&headers));
                return StatusCode::OK;
            }
        }
    }
    Metrics::inc(&st.metrics.gate_deny);
    StatusCode::UNAUTHORIZED
}

/// Check an Access assertion against the cached keys. The lock is held only inside this call, so
/// it is never held across an await.
fn check_access(access: &AccessState, jwt: &str, now: u64) -> Result<AccessIdentity, AccessError> {
    let jwks = access.jwks.read().map_err(|_| AccessError::BadSignature)?;
    verify_access_jwt_identity(jwt, &jwks, &access.aud, &access.issuer, now)
}

pub async fn logout(State(st): State<AppState>, headers: HeaderMap) -> Response {
    st.auditor.record(
        AuditEvent::new(now_ms(), event::LOGOUT, outcome::SUCCESS)
            .with("ip", client_ip(&headers))
            .with_opt("ua", user_agent(&headers).as_deref()),
    );
    (
        StatusCode::SEE_OTHER,
        [
            (header::SET_COOKIE, clear_cookie(secure_flag(&headers))),
            (header::LOCATION, "/auth/login".to_string()),
        ],
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::{clear_cookie, safe_next, secure_flag, set_cookie};
    use axum::http::HeaderMap;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, v.parse().unwrap());
        }
        h
    }

    #[test]
    fn the_cookie_is_secure_only_when_the_client_came_over_https() {
        assert_eq!(secure_flag(&headers(&[])), "");
        assert_eq!(secure_flag(&headers(&[("x-forwarded-proto", "http")])), "");
        assert_eq!(
            secure_flag(&headers(&[("x-forwarded-proto", "https")])),
            "; Secure"
        );
        assert_eq!(
            secure_flag(&headers(&[("cf-visitor", r#"{"scheme":"https"}"#)])),
            "; Secure"
        );
        let secure = set_cookie("tok", 60, "; Secure");
        assert_eq!(
            secure,
            "sdw_session=tok; HttpOnly; SameSite=Lax; Path=/; Secure; Max-Age=60"
        );
        assert!(set_cookie("tok", 60, "").ends_with("Path=/; Max-Age=60"));
        // Clearing must carry the same attributes, or a browser may keep the original cookie.
        assert!(clear_cookie("; Secure").contains("; Secure; Max-Age=0"));
    }

    #[test]
    fn local_paths_are_kept() {
        assert_eq!(safe_next(Some("/sync/")), "/sync/");
        assert_eq!(safe_next(Some("/a?b=c&d=e")), "/a?b=c&d=e");
        assert_eq!(safe_next(Some("/")), "/");
    }

    #[test]
    fn anything_a_browser_could_read_as_another_site_is_dropped() {
        for bad in [
            "//evil.example",
            "//evil.example/x",
            "/\\evil.example",
            "/\\/evil.example",
            "/a\\b",
            "/\t/evil.example",
            "/\n/evil.example",
            "/\r/evil.example",
            "https://evil.example",
            "evil.example",
            "javascript:alert(1)",
            "",
        ] {
            assert_eq!(safe_next(Some(bad)), "/", "{bad:?} must not survive");
        }
        assert_eq!(safe_next(None), "/");
    }
}
