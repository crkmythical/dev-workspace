//! HTTP-level tests of the graduated surface: the feature gates, the doctor
//! shape, listing auth, and the whole remote-destruct decision path with a fake
//! trigger (so the irreversible step never runs). Mirrors the status codes
//! tests/smoke/checks/20-auth.sh and 80-destruct.sh assert against the real image.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use sdw_core::sync::aad::build_aad;
use sdw_core::sync::crypto::{derive_key, encrypt};
use sdw_core::sync::protocol::PNG_HEADER;
use serde_json::Value;
use sync_service::destruct::DestructTrigger;
use sync_service::features::Features;
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
    _dir: tempfile::TempDir,
}

fn harness(features: Features) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let fired = Arc::new(AtomicBool::new(false));
    let cfg = AppConfig {
        features,
        dirs: Dirs {
            shared: p.join("shared").to_string_lossy().into_owned(),
            upload: p.join("uploads").to_string_lossy().into_owned(),
        },
        passphrase_path: p.join("sync-passphrase").to_string_lossy().into_owned(),
        key_hash_path: p.join("key-hash").to_string_lossy().into_owned(),
        destruct: Arc::new(Flag(fired.clone())),
    };
    Harness {
        state: AppState::from_config(cfg),
        fired,
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
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "destroyed");
    // The trigger is launched synchronously before the response, so a failure to start it would
    // have been a 500 (see destruct_reports_an_error_when_the_trigger_cannot_launch).
    assert!(h.fired.load(Ordering::SeqCst), "the destruct trigger ran");
}

#[tokio::test]
async fn destruct_reports_an_error_when_the_trigger_cannot_launch() {
    // A right passphrase but the destroyer cannot be started: the route must NOT answer
    // "destroyed". It reports 500 instead of hiding the failure behind an optimistic 200.
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
    let meta =
        format!(r#"{{"file_id":"fileABC","file_path":"out.txt","content_sha256":"{digest}"}}"#);
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
