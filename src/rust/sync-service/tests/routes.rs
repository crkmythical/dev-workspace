//! HTTP-level tests of the graduated surface: the feature gates, the doctor
//! shape, listing auth, and the whole remote-destruct decision path with a fake
//! trigger (so the irreversible step never runs). Mirrors the status codes
//! tests/smoke/checks/20-auth.sh and 80-destruct.sh assert against the real image.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use sdw_core::audit::{parse_line, MemorySink, ParsedEvent};
use sdw_core::sync::aad::build_aad;
use sdw_core::sync::crypto::{derive_key, encrypt};
use sdw_core::sync::protocol::PNG_HEADER;
use serde_json::Value;
use sync_service::destruct::DestructTrigger;
use sync_service::features::Features;
use sync_service::state::AuthState;
use sync_service::store::Dirs;
use sync_service::util::now_ms;
use sync_service::{app, AppConfig, AppState};
use tower::ServiceExt;

/// A bcrypt hash of PASSPHRASE produced by Bun.password (cost 8, for a fast test).
/// Pinned alongside tests/bcrypt_parity.rs: Bun <-> the bcrypt crate.
const PASSPHRASE: &str = "correct horse battery staple";
const PASSPHRASE_HASH: &str = "$2b$08$efMja3NPRA/pb4MDgnmf.eCXOE56IzoWLpNS7VUjMCRFKM.GdaB3G";

struct Flag(Arc<AtomicBool>);
impl DestructTrigger for Flag {
    fn trigger(&self) -> Result<(), String> {
        self.0.store(true, Ordering::SeqCst);
        Ok(())
    }
}

struct Harness {
    state: AppState,
    fired: Arc<AtomicBool>,
    /// Every audit line the service wrote.
    sink: Arc<MemorySink>,
    _dir: tempfile::TempDir,
}

impl Harness {
    fn audit(&self) -> Vec<ParsedEvent> {
        self.sink
            .lines()
            .iter()
            .map(|l| parse_line(l).expect("an audit line parses"))
            .collect()
    }
}

fn harness(features: Features) -> Harness {
    harness_with(features, false)
}

fn harness_with(features: Features, metrics_enabled: bool) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let fired = Arc::new(AtomicBool::new(false));
    let sink = Arc::new(MemorySink::new());
    let cfg = AppConfig {
        features,
        dirs: Dirs {
            shared: p.join("shared").to_string_lossy().into_owned(),
            upload: p.join("uploads").to_string_lossy().into_owned(),
        },
        passphrase_path: p.join("sync-passphrase").to_string_lossy().into_owned(),
        key_hash_path: p.join("key-hash").to_string_lossy().into_owned(),
        destruct: Arc::new(Flag(fired.clone())),
        auth: AuthState {
            session_key: Arc::new([7u8; 32]),
            password_hash: Some(Arc::from(PASSPHRASE_HASH)),
            ttl_secs: 3600,
            access: None,
        },
        audit: sink.clone(),
        metrics_enabled,
    };
    Harness {
        state: AppState::from_config(cfg),
        fired,
        sink,
        _dir: dir,
    }
}

fn both_on() -> Features {
    Features {
        sync: true,
        remote_destruct: true,
    }
}

fn write(path: &std::sync::Arc<str>, contents: &str) {
    std::fs::write(&**path, contents).unwrap();
}

async fn send(state: &AppState, req: Request<Body>) -> (StatusCode, Value) {
    let resp = app(state.clone()).oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body)
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn post_json(uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

// ── doctor ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn doctor_reports_disabled_when_sync_off() {
    let h = harness(Features {
        sync: false,
        remote_destruct: false,
    });
    let (status, body) = send(&h.state, get("/sync/api/doctor")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["state"], "disabled");
    assert_eq!(body["sync_enabled"], false);
}

#[tokio::test]
async fn doctor_reports_vault_locked_without_a_key() {
    let h = harness(both_on());
    let (_, body) = send(&h.state, get("/sync/api/doctor")).await;
    assert_eq!(body["state"], "vault-locked");
    assert_eq!(body["sync_key_available"], false);
}

#[tokio::test]
async fn doctor_reports_ready_with_a_key() {
    let h = harness(both_on());
    write(&h.state.passphrase_path, "the vault passphrase");
    let (_, body) = send(&h.state, get("/sync/api/doctor")).await;
    assert_eq!(body["state"], "ready");
    assert_eq!(body["sync_key_available"], true);
}

// ── feature gates ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn upload_and_download_are_404_when_sync_off() {
    let h = harness(Features {
        sync: false,
        remote_destruct: true,
    });
    let (up, _) = send(&h.state, post_json("/sync/api/upload", "")).await;
    assert_eq!(up, StatusCode::NOT_FOUND);
    let (down, _) = send(&h.state, get("/sync/api/download")).await;
    assert_eq!(down, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn download_without_proof_is_unauthorized() {
    let h = harness(both_on());
    write(&h.state.passphrase_path, "the vault passphrase");
    let (status, _) = send(&h.state, get("/sync/api/download")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn download_with_a_path_query_is_404() {
    let h = harness(both_on());
    write(&h.state.passphrase_path, "the vault passphrase");
    let (status, _) = send(&h.state, get("/sync/api/download?path=secret")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── remote destruct ───────────────────────────────────────────────────────────

#[tokio::test]
async fn destruct_is_404_when_the_feature_is_off() {
    let h = harness(Features {
        sync: true,
        remote_destruct: false,
    });
    write(&h.state.key_hash_path, PASSPHRASE_HASH);
    let (status, _) = send(
        &h.state,
        post_json("/sync/api/destruct", r#"{"passphrase":"x"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!h.fired.load(Ordering::SeqCst));
}

#[tokio::test]
async fn destruct_is_404_without_a_key_hash() {
    let h = harness(both_on());
    let (status, _) = send(
        &h.state,
        post_json("/sync/api/destruct", r#"{"passphrase":"x"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn destruct_is_404_for_a_wrong_passphrase_or_no_body() {
    let h = harness(both_on());
    write(&h.state.key_hash_path, PASSPHRASE_HASH);
    let (wrong, _) = send(
        &h.state,
        post_json("/sync/api/destruct", r#"{"passphrase":"nope"}"#),
    )
    .await;
    assert_eq!(wrong, StatusCode::NOT_FOUND);
    let (nobody, _) = send(&h.state, post_json("/sync/api/destruct", "")).await;
    assert_eq!(nobody, StatusCode::NOT_FOUND);
    assert!(!h.fired.load(Ordering::SeqCst));
}

#[tokio::test]
async fn destruct_fires_on_the_right_passphrase() {
    let h = harness(both_on());
    write(&h.state.key_hash_path, PASSPHRASE_HASH);
    let body = format!(r#"{{"passphrase":"{PASSPHRASE}"}}"#);
    let (status, body) = send(&h.state, post_json("/sync/api/destruct", &body)).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body["status"], "scheduled");
    // The worker is launched before the 202 response; completion is deliberately not claimed.
    assert!(h.fired.load(Ordering::SeqCst), "the destruct trigger ran");
}

#[tokio::test]
async fn destruct_reports_an_error_when_the_trigger_cannot_launch() {
    // A right passphrase but the destroyer cannot be started: the route must not answer
    // "scheduled". It reports 500 and allows a later retry.
    struct Failing;
    impl DestructTrigger for Failing {
        fn trigger(&self) -> Result<(), String> {
            Err("could not launch self-destruct".to_string())
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let cfg = AppConfig {
        features: both_on(),
        dirs: Dirs {
            shared: p.join("shared").to_string_lossy().into_owned(),
            upload: p.join("uploads").to_string_lossy().into_owned(),
        },
        passphrase_path: p.join("sync-passphrase").to_string_lossy().into_owned(),
        key_hash_path: p.join("key-hash").to_string_lossy().into_owned(),
        destruct: Arc::new(Failing),
        auth: AuthState {
            session_key: Arc::new([7u8; 32]),
            password_hash: Some(Arc::from(PASSPHRASE_HASH)),
            ttl_secs: 3600,
            access: None,
        },
        audit: Arc::new(MemorySink::new()),
        metrics_enabled: false,
    };
    let state = AppState::from_config(cfg);
    write(&state.key_hash_path, PASSPHRASE_HASH);
    let body = format!(r#"{{"passphrase":"{PASSPHRASE}"}}"#);
    let (status, out) = send(&state, post_json("/sync/api/destruct", &body)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(out["status"], "error");
}

#[tokio::test]
async fn destruct_rate_limits_the_global_budget() {
    let h = harness(both_on());
    // No key hash: every call is a 404 until the global cap (10) trips at the 11th.
    let mut codes = Vec::new();
    for n in 0..11 {
        let body = format!(r#"{{"passphrase":"rotate-{n}"}}"#);
        let req = Request::builder()
            .method("POST")
            .uri("/sync/api/destruct")
            .header("content-type", "application/json")
            .header("cf-connecting-ip", format!("10.0.0.{n}"))
            .body(Body::from(body))
            .unwrap();
        let (status, _) = send(&h.state, req).await;
        codes.push(status);
    }
    assert!(codes[..10].iter().all(|c| *c == StatusCode::NOT_FOUND));
    assert_eq!(codes[10], StatusCode::TOO_MANY_REQUESTS);
}

// ── upload happy path (real encrypted bytes through the handler + store) ───────

/// Frame an encrypted body the way the browser client does: PNG header, nonce,
/// then ciphertext||tag.
fn body(key: &[u8], nonce: &[u8; 12], plaintext: &[u8], aad: &[u8]) -> Vec<u8> {
    let ct = encrypt(key, nonce, plaintext, aad).unwrap();
    let mut out = PNG_HEADER.to_vec();
    out.extend_from_slice(nonce);
    out.extend_from_slice(&ct);
    out
}

fn upload_req(op: &str, ts: i64, file_id: &str, chunk_idx: &str, body: Vec<u8>) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/sync/api/upload")
        .header("x-sync-op", op)
        .header("x-sync-ts", ts.to_string())
        .header("x-sync-fileid", file_id)
        .header("x-sync-chunkidx", chunk_idx)
        .body(Body::from(body))
        .unwrap()
}

#[tokio::test]
async fn upload_accepts_a_chunk_larger_than_axums_2mib_default() {
    // Regression: the browser uploads up to CHUNK_SIZE = 4 MiB per request, but axum's default
    // `Bytes` limit is 2 MiB. Without app() raising DefaultBodyLimit, a real chunk is rejected
    // 413 before the handler and files over ~2 MB silently fail to upload. Prove a 3 MiB chunk
    // (over the 2 MiB default, under the 5 MiB cap) is accepted end to end.
    let h = harness(both_on());
    write(&h.state.passphrase_path, "the vault passphrase");
    let key = derive_key("the vault passphrase");
    let ts = now_ms();
    let plaintext = vec![0u8; 3 * 1024 * 1024];
    let aad = build_aad("put-chunk", ts, &["bigfile", "0"]);
    let req = upload_req(
        "put-chunk",
        ts,
        "bigfile",
        "0",
        body(&key, &[9u8; 12], &plaintext, &aad),
    );
    let (status, out) = send(&h.state, req).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a 3 MiB chunk must not be 413'd: {out:?}"
    );
    assert_eq!(out["status"], "chunk-received");
}

#[tokio::test]
async fn upload_with_a_stale_timestamp_says_stale_not_replay() {
    // An authentic envelope with an out-of-tolerance timestamp is a clock/stale problem, and now
    // reports "stale-timestamp" rather than the misleading "replay-detected" it used to.
    let h = harness(both_on());
    write(&h.state.passphrase_path, "the vault passphrase");
    let key = derive_key("the vault passphrase");
    let ts = now_ms() - 10 * 60_000; // 10 min old, past the 5 min tolerance
    let aad = build_aad("put-chunk", ts, &["f", "0"]);
    let req = upload_req(
        "put-chunk",
        ts,
        "f",
        "0",
        body(&key, &[3u8; 12], b"x", &aad),
    );
    let (status, out) = send(&h.state, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(out["error"], "stale-timestamp");
}

#[tokio::test]
async fn upload_chunk_then_finalize_lands_the_file() {
    let h = harness(both_on());
    write(&h.state.passphrase_path, "the vault passphrase");
    let key = derive_key("the vault passphrase");
    let plaintext = b"hello from the sync page";

    // 1. put-chunk, authenticated under its own AAD.
    let ts1 = now_ms();
    let chunk_aad = build_aad("put-chunk", ts1, &["fileABC", "0"]);
    let req = upload_req(
        "put-chunk",
        ts1,
        "fileABC",
        "0",
        body(&key, &[1u8; 12], plaintext, &chunk_aad),
    );
    let (status, out) = send(&h.state, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(out["status"], "chunk-received");

    // The very same body again is a replay (same nonce) and is refused.
    let req = upload_req(
        "put-chunk",
        ts1,
        "fileABC",
        "0",
        body(&key, &[1u8; 12], plaintext, &chunk_aad),
    );
    let (replay, _) = send(&h.state, req).await;
    assert_eq!(replay, StatusCode::BAD_REQUEST);

    // 2. put-finalize with the matching checksum commits it into the shared dir.
    let ts2 = ts1 + 1;
    let digest = {
        use sha2::{Digest, Sha256};
        let mut hsh = Sha256::new();
        hsh.update(plaintext);
        hex::encode(hsh.finalize())
    };
    let meta = format!(
        r#"{{"file_id":"fileABC","file_path":"out.txt","content_sha256":"{digest}","total_chunks":1}}"#
    );
    let final_aad = build_aad("put-finalize", ts2, &["fileABC", "1"]);
    let req = upload_req(
        "put-finalize",
        ts2,
        "fileABC",
        "1",
        body(&key, &[2u8; 12], meta.as_bytes(), &final_aad),
    );
    let (status, out) = send(&h.state, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(out["status"], "upload-complete");
    assert_eq!(out["path"], "out.txt");

    let landed = std::fs::read(format!("{}/out.txt", h.state.dirs.shared)).unwrap();
    assert_eq!(
        landed, plaintext,
        "the decrypted bytes are what the shared dir holds"
    );
}

#[tokio::test]
async fn upload_with_a_wrong_key_fails_to_authenticate() {
    let h = harness(both_on());
    write(&h.state.passphrase_path, "the vault passphrase");
    let wrong = derive_key("some other passphrase");
    let ts = now_ms();
    let aad = build_aad("put-chunk", ts, &["f", "0"]);
    let req = upload_req(
        "put-chunk",
        ts,
        "f",
        "0",
        body(&wrong, &[3u8; 12], b"x", &aad),
    );
    let (status, out) = send(&h.state, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(out["error"], "aead-failure");
}

#[tokio::test]
async fn finalize_without_a_checksum_is_rejected() {
    // The client always sends content_sha256; a finalize that omits it is malformed and must not
    // commit unverified bytes. (Guards the integrity fix: the server used to accept it.)
    let h = harness(both_on());
    write(&h.state.passphrase_path, "the vault passphrase");
    let key = derive_key("the vault passphrase");

    // A chunk first, so finalize gets past the has-chunks check and reaches the checksum gate.
    let ts1 = now_ms();
    let chunk_aad = build_aad("put-chunk", ts1, &["fileNoSum", "0"]);
    let (status, _) = send(
        &h.state,
        upload_req(
            "put-chunk",
            ts1,
            "fileNoSum",
            "0",
            body(&key, &[7u8; 12], b"bytes", &chunk_aad),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Finalize WITHOUT content_sha256.
    let ts2 = ts1 + 1;
    let meta = r#"{"file_id":"fileNoSum","file_path":"x.txt","total_chunks":1}"#;
    let final_aad = build_aad("put-finalize", ts2, &["fileNoSum", "1"]);
    let (status, out) = send(
        &h.state,
        upload_req(
            "put-finalize",
            ts2,
            "fileNoSum",
            "1",
            body(&key, &[8u8; 12], meta.as_bytes(), &final_aad),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(out["error"], "missing-checksum");
}

// --- unified auth gateway ---------------------------------------------------------------------

#[tokio::test]
async fn gate_rejects_without_cookie_and_accepts_after_login() {
    use axum::http::header::{CONTENT_TYPE, COOKIE, LOCATION, SET_COOKIE};
    let h = harness(both_on());

    // No cookie -> the gate denies (what Caddy forward_auth turns into a login redirect / WS fail).
    let resp = app(h.state.clone())
        .oneshot(Request::get("/auth/verify").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Wrong password -> 401 and no cookie issued.
    let resp = app(h.state.clone())
        .oneshot(
            Request::post("/auth/login")
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("password=wrong&next=/"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert!(resp.headers().get(SET_COOKIE).is_none());

    // Right password -> 303 to a safe `next`, with the session cookie.
    let resp = app(h.state.clone())
        .oneshot(
            Request::post("/auth/login")
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "password={}&next=/vnc/",
                    PASSPHRASE.replace(' ', "+")
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(resp.headers().get(LOCATION).unwrap(), "/vnc/");
    let set = resp
        .headers()
        .get(SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(set.starts_with("sdw_session=v1."), "{set}");
    let token = set["sdw_session=".len()..]
        .split(';')
        .next()
        .unwrap()
        .to_string();

    // That cookie satisfies the gate — this is exactly what a browser replays on the WS upgrade.
    let resp = app(h.state.clone())
        .oneshot(
            Request::get("/auth/verify")
                .header(COOKIE, format!("sdw_session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn open_redirect_in_next_is_neutralized() {
    use axum::http::header::{CONTENT_TYPE, LOCATION};
    let h = harness(both_on());
    let resp = app(h.state.clone())
        .oneshot(
            Request::post("/auth/login")
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "password={}&next=//evil.example/x",
                    PASSPHRASE.replace(' ', "+")
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    // Protocol-relative target rejected -> falls back to "/".
    assert_eq!(resp.headers().get(LOCATION).unwrap(), "/");
}

#[tokio::test]
async fn logout_clears_cookie() {
    use axum::http::header::SET_COOKIE;
    let h = harness(both_on());
    let resp = app(h.state.clone())
        .oneshot(Request::post("/auth/logout").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let c = resp.headers().get(SET_COOKIE).unwrap().to_str().unwrap();
    assert!(c.contains("sdw_session=;"), "{c}");
    assert!(c.contains("Max-Age=0"), "{c}");
}

/// A session cookie value for the harness's key, as a browser would replay it.
fn session_cookie() -> String {
    use sdw_core::auth::session::sign_session;
    let now = now_ms().max(0) as u64 / 1000;
    format!("sdw_session={}", sign_session(&[7u8; 32], now, now + 3600))
}

#[tokio::test]
async fn the_session_cookie_is_secure_only_behind_https() {
    use axum::http::header::SET_COOKIE;
    let h = harness(both_on());
    let cookie_of = |resp: &axum::http::Response<Body>| {
        resp.headers()
            .get(SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string()
    };
    // Plain HTTP on localhost: no `Secure`, or Safari would not keep it.
    let plain = app(h.state.clone())
        .oneshot(login_req(PASSPHRASE, "/", "198.51.100.1"))
        .await
        .unwrap();
    assert!(
        !cookie_of(&plain).contains("Secure"),
        "{}",
        cookie_of(&plain)
    );
    // Through the HTTPS edge (the tunnel reports the client's scheme): `Secure`.
    let mut req = login_req(PASSPHRASE, "/", "198.51.100.2");
    req.headers_mut()
        .insert("x-forwarded-proto", "https".parse().unwrap());
    let secure = app(h.state.clone()).oneshot(req).await.unwrap();
    let c = cookie_of(&secure);
    assert!(
        c.contains("; Secure;") && c.contains("HttpOnly") && c.contains("SameSite=Lax"),
        "{c}"
    );
    // And logging out clears it with the same attributes.
    let out = app(h.state.clone())
        .oneshot(
            Request::post("/auth/logout")
                .header("x-forwarded-proto", "https")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(cookie_of(&out).contains("; Secure;"), "{}", cookie_of(&out));
}

#[tokio::test]
async fn a_websocket_handshake_from_another_origin_is_refused_whatever_credential_it_carries() {
    use axum::http::header::COOKIE;
    let h = harness(both_on());
    let verify = |origin: Option<&'static str>, upgrade: bool| {
        let mut r = Request::get("/auth/verify")
            .header(COOKIE, session_cookie())
            .header("host", "ws.example.com");
        if let Some(o) = origin {
            r = r.header("origin", o);
        }
        if upgrade {
            r = r
                .header("upgrade", "websocket")
                .header("connection", "Upgrade");
        }
        r.body(Body::empty()).unwrap()
    };
    let status = |req: Request<Body>| {
        let state = h.state.clone();
        async move { app(state).oneshot(req).await.unwrap().status() }
    };
    // The page that owns the host: allowed.
    assert_eq!(
        status(verify(Some("https://ws.example.com"), true)).await,
        StatusCode::OK
    );
    // A sibling subdomain shares the cookie's SITE but is not the same origin.
    assert_eq!(
        status(verify(Some("https://other.example.com"), true)).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status(verify(Some("null"), true)).await,
        StatusCode::FORBIDDEN
    );
    // Not a browser (no Origin), or not a WebSocket: the cookie decides as before.
    assert_eq!(status(verify(None, true)).await, StatusCode::OK);
    assert_eq!(
        status(verify(Some("https://other.example.com"), false)).await,
        StatusCode::OK
    );
    // The refusal is counted and audited (with the origin that tried).
    let events = h.audit();
    let refused: Vec<_> = events.iter().filter(|e| e.event == "gate.origin").collect();
    assert_eq!(refused.len(), 2, "{events:?}");
    assert_eq!(
        refused[0].field("origin"),
        Some("https://other.example.com")
    );
    assert_eq!(refused[0].field("host"), Some("ws.example.com"));
}

// --- audit trail, login throttle, metrics --------------------------------------------------------

fn login_req(password: &str, next: &str, ip: &str) -> Request<Body> {
    use axum::http::header::CONTENT_TYPE;
    Request::post("/auth/login")
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header("cf-connecting-ip", ip)
        .header("user-agent", "smoke-agent/1.0")
        .body(Body::from(format!(
            "password={}&next={next}",
            password.replace(' ', "+")
        )))
        .unwrap()
}

#[tokio::test]
async fn logins_are_audited_with_who_when_and_result_but_never_the_password() {
    let h = harness(both_on());
    let wrong = app(h.state.clone())
        .oneshot(login_req("hunter2-not-it", "/", "203.0.113.9"))
        .await
        .unwrap();
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    let right = app(h.state.clone())
        .oneshot(login_req(PASSPHRASE, "/", "203.0.113.9"))
        .await
        .unwrap();
    assert_eq!(right.status(), StatusCode::SEE_OTHER);

    let events = h.audit();
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(
        (events[0].event.as_str(), events[0].outcome.as_str()),
        ("auth.login", "failure")
    );
    assert_eq!(
        (events[1].event.as_str(), events[1].outcome.as_str()),
        ("auth.login", "success")
    );
    for e in &events {
        assert_eq!(e.field("ip"), Some("203.0.113.9"));
        assert_eq!(e.field("ua"), Some("smoke-agent/1.0"));
        assert!(e.ts_ms > 1_700_000_000_000);
    }
    // The secret itself is nowhere in what was written.
    let raw = h.sink.lines().join("\n");
    assert!(!raw.contains("hunter2"), "{raw}");
    assert!(!raw.contains(&PASSPHRASE.replace(' ', "+")), "{raw}");
    assert!(!raw.contains(PASSPHRASE), "{raw}");
}

#[tokio::test]
async fn logins_are_throttled_per_address_and_the_throttle_is_audited() {
    let h = harness(both_on());
    // 10 attempts are allowed per address per minute (wrong ones: nothing is gained from a right one).
    for n in 0..10 {
        let r = app(h.state.clone())
            .oneshot(login_req(&format!("guess-{n}"), "/", "198.51.100.5"))
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "attempt {n}");
    }
    // The 11th is refused before any bcrypt work, even with the right password.
    let blocked = app(h.state.clone())
        .oneshot(login_req(PASSPHRASE, "/", "198.51.100.5"))
        .await
        .unwrap();
    assert_eq!(blocked.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(blocked.headers().get("retry-after").unwrap(), "60");
    assert!(blocked.headers().get("set-cookie").is_none());
    // Another address has its own budget.
    let other = app(h.state.clone())
        .oneshot(login_req(PASSPHRASE, "/", "198.51.100.6"))
        .await
        .unwrap();
    assert_eq!(other.status(), StatusCode::SEE_OTHER);

    let events = h.audit();
    assert!(events
        .iter()
        .any(|e| e.outcome == "throttled" && e.field("ip") == Some("198.51.100.5")));
}

#[tokio::test]
async fn the_login_page_is_not_frameable_cacheable_or_scriptable() {
    let h = harness(both_on());
    let resp = app(h.state.clone())
        .oneshot(
            Request::get("/auth/login?next=/sync/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let hdr = |n: &str| resp.headers().get(n).unwrap().to_str().unwrap().to_string();
    assert_eq!(hdr("x-frame-options"), "DENY");
    assert_eq!(hdr("cache-control"), "no-store");
    assert_eq!(hdr("x-content-type-options"), "nosniff");
    assert_eq!(hdr("referrer-policy"), "no-referrer");
    let csp = hdr("content-security-policy");
    assert!(
        csp.contains("default-src 'none'") && csp.contains("frame-ancestors 'none'"),
        "{csp}"
    );
}

#[tokio::test]
async fn a_backslash_open_redirect_in_next_is_neutralized() {
    use axum::http::header::LOCATION;
    let h = harness(both_on());
    for evil in ["/%5Cevil.example", "//evil.example", "/\\evil.example"] {
        // The percent-encoded form arrives decoded by the form parser, as `/\evil.example`.
        let resp = app(h.state.clone())
            .oneshot(login_req(PASSPHRASE, evil, "192.0.2.1"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER, "{evil}");
        assert_eq!(resp.headers().get(LOCATION).unwrap(), "/", "{evil}");
    }
}

#[tokio::test]
async fn logout_is_audited() {
    let h = harness(both_on());
    let _ = app(h.state.clone())
        .oneshot(
            Request::post("/auth/logout")
                .header("x-forwarded-for", "192.0.2.77, 10.0.0.1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let events = h.audit();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event, "auth.logout");
    assert_eq!(events[0].field("ip"), Some("192.0.2.77"));
}

#[tokio::test]
async fn destruct_attempts_are_audited_by_reason_and_a_fired_one_is_always_written() {
    let h = harness(both_on());
    write(&h.state.key_hash_path, PASSPHRASE_HASH);
    let req = |pass: &str| {
        Request::builder()
            .method("POST")
            .uri("/sync/api/destruct")
            .header("content-type", "application/json")
            .header("cf-connecting-ip", "203.0.113.50")
            .body(Body::from(format!(r#"{{"passphrase":"{pass}"}}"#)))
            .unwrap()
    };
    let (s, _) = send(&h.state, req("nope")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = send(&h.state, req(PASSPHRASE)).await;
    assert_eq!(s, StatusCode::ACCEPTED);
    assert!(h.fired.load(Ordering::SeqCst));

    let events = h.audit();
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(
        (events[0].event.as_str(), events[0].outcome.as_str()),
        ("destruct.attempt", "rejected")
    );
    assert_eq!(events[0].field("reason"), Some("wrong_passphrase"));
    assert_eq!(
        (events[1].event.as_str(), events[1].outcome.as_str()),
        ("destruct.attempt", "triggered")
    );
    assert_eq!(events[1].field("ip"), Some("203.0.113.50"));
    let raw = h.sink.lines().join("\n");
    assert!(!raw.contains(PASSPHRASE) && !raw.contains("nope"), "{raw}");
}

#[tokio::test]
async fn a_disabled_destruct_endpoint_leaves_no_trace_in_the_log() {
    let h = harness(Features {
        sync: true,
        remote_destruct: false,
    });
    let (s, _) = send(
        &h.state,
        post_json("/sync/api/destruct", r#"{"passphrase":"x"}"#),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(h.audit().is_empty());
}

#[tokio::test]
async fn metrics_are_a_404_unless_enabled_and_then_count_the_gate_and_logins() {
    let off = harness(both_on());
    let resp = app(off.state.clone())
        .oneshot(get("/sync/api/metrics"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let on = harness_with(both_on(), true);
    // One gate denial, one wrong login, one right login, one cookie pass.
    let _ = app(on.state.clone())
        .oneshot(get("/auth/verify"))
        .await
        .unwrap();
    let _ = app(on.state.clone())
        .oneshot(login_req("wrong", "/", "192.0.2.9"))
        .await
        .unwrap();
    let ok = app(on.state.clone())
        .oneshot(login_req(PASSPHRASE, "/", "192.0.2.9"))
        .await
        .unwrap();
    let cookie = ok.headers().get("set-cookie").unwrap().to_str().unwrap();
    let token = cookie.split(';').next().unwrap().to_string();
    let _ = app(on.state.clone())
        .oneshot(
            Request::get("/auth/verify")
                .header("cookie", token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let resp = app(on.state.clone())
        .oneshot(get("/sync/api/metrics"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("text/plain"));
    let text = String::from_utf8(
        to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(
        text.contains("sdw_login_total{outcome=\"failure\"} 1"),
        "{text}"
    );
    assert!(
        text.contains("sdw_login_total{outcome=\"success\"} 1"),
        "{text}"
    );
    assert!(
        text.contains("sdw_gate_requests_total{result=\"deny\"} 1"),
        "{text}"
    );
    assert!(
        text.contains("sdw_gate_requests_total{result=\"allow_cookie\"} 1"),
        "{text}"
    );
}
