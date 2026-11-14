//! Cloudflare rotates the keys that sign Access assertions. The gate keeps the team's keys cached
//! and refreshes them hourly, so an assertion signed by a NEW key would be refused for up to an hour
//! unless the gate fetches the keys again when it meets a key id it does not know. It does, at most
//! once a minute, so a stream of forged key ids cannot make it fetch without limit.
//!
//! The "team certs" are a file:// URL here (the production fetch is curl, which reads both).

use std::sync::atomic::AtomicI64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rsa::pkcs1v15::SigningKey;
use rsa::signature::{SignatureEncoding, Signer};
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use sdw_core::audit::MemorySink;
use sdw_core::auth::access_jwt::Jwks;
use sha2::Sha256;
use sync_service::destruct::DestructTrigger;
use sync_service::features::Features;
use sync_service::state::{AccessState, AuthState};
use sync_service::store::Dirs;
use sync_service::util::now_ms;
use sync_service::{app, AppConfig, AppState};
use tower::ServiceExt;

const AUD: &str = "rotation-aud";
const ISS: &str = "https://team.cloudflareaccess.com";

struct NoDestruct(AtomicBool);
impl DestructTrigger for NoDestruct {
    fn trigger(&self) -> Result<(), String> {
        self.0.store(true, Ordering::SeqCst);
        Ok(())
    }
}

/// Three throwaway keys for the whole test binary (generation is the slow part).
fn keys() -> &'static [RsaPrivateKey; 3] {
    static K: OnceLock<[RsaPrivateKey; 3]> = OnceLock::new();
    K.get_or_init(|| {
        let make = || RsaPrivateKey::new(&mut rand_core::OsRng, 2048).unwrap();
        [make(), make(), make()]
    })
}

fn jwk_json(kid: &str, key: &RsaPrivateKey) -> String {
    let public = key.to_public_key();
    format!(
        r#"{{"kid":"{kid}","kty":"RSA","alg":"RS256","use":"sig","n":"{}","e":"{}"}}"#,
        URL_SAFE_NO_PAD.encode(public.n().to_bytes_be()),
        URL_SAFE_NO_PAD.encode(public.e().to_bytes_be())
    )
}

fn jwks_json(entries: &[(&str, &RsaPrivateKey)]) -> String {
    let list: Vec<String> = entries.iter().map(|(k, key)| jwk_json(k, key)).collect();
    format!(r#"{{"keys":[{}]}}"#, list.join(","))
}

/// An assertion for this app, signed by `key` under `kid`, valid for ten minutes.
fn assertion(kid: &str, key: &RsaPrivateKey) -> String {
    let exp = now_ms() / 1000 + 600;
    let header = URL_SAFE_NO_PAD.encode(format!(r#"{{"alg":"RS256","kid":"{kid}","typ":"JWT"}}"#));
    let claims = URL_SAFE_NO_PAD.encode(format!(
        r#"{{"aud":["{AUD}"],"iss":"{ISS}","exp":{exp},"email":"me@example.com"}}"#
    ));
    let input = format!("{header}.{claims}");
    let sig = SigningKey::<Sha256>::new(key.clone()).sign(input.as_bytes());
    format!("{input}.{}", URL_SAFE_NO_PAD.encode(sig.to_bytes()))
}

struct Gate {
    state: AppState,
    access: AccessState,
    certs: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

/// A gate that has cached only the key "k1", whose team certs URL is a file the test rewrites.
fn gate() -> Gate {
    let dir = tempfile::tempdir().unwrap();
    let certs = dir.path().join("certs.json");
    let [k1, ..] = keys();
    std::fs::write(&certs, jwks_json(&[("k1", k1)])).unwrap();
    let cached: Jwks = serde_json::from_str(&jwks_json(&[("k1", k1)])).unwrap();
    let access = AccessState {
        aud: Arc::from(AUD),
        issuer: Arc::from(ISS),
        certs_url: Arc::from(format!("file://{}", certs.display())),
        jwks: Arc::new(RwLock::new(cached)),
        last_forced_refetch_ms: Arc::new(AtomicI64::new(0)),
    };
    let cfg = AppConfig {
        features: Features {
            sync: false,
            remote_destruct: false,
        },
        dirs: Dirs {
            shared: dir.path().join("shared").to_string_lossy().into_owned(),
            upload: dir.path().join("uploads").to_string_lossy().into_owned(),
        },
        passphrase_path: dir.path().join("pass").to_string_lossy().into_owned(),
        key_hash_path: dir.path().join("hash").to_string_lossy().into_owned(),
        destruct: Arc::new(NoDestruct(AtomicBool::new(false))),
        auth: AuthState {
            session_key: Arc::new([9u8; 32]),
            password_hash: None,
            ttl_secs: 3600,
            access: Some(access.clone()),
        },
        audit: Arc::new(MemorySink::new()),
        metrics_enabled: false,
    };
    Gate {
        state: AppState::from_config(cfg),
        access,
        certs,
        _dir: dir,
    }
}

async fn verify(gate: &Gate, jwt: &str) -> StatusCode {
    app(gate.state.clone())
        .oneshot(
            Request::get("/auth/verify")
                .header("cf-access-jwt-assertion", jwt)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn an_assertion_from_a_rotated_in_key_is_accepted_after_one_refetch() {
    let g = gate();
    let [k1, k2, _] = keys();
    // A key the cache already has needs no fetch.
    assert_eq!(verify(&g, &assertion("k1", k1)).await, StatusCode::OK);
    assert_eq!(g.access.last_forced_refetch_ms.load(Ordering::Relaxed), 0);

    // Cloudflare rotated: the team's certs now list k2 as well, but the cache has only k1.
    std::fs::write(&g.certs, jwks_json(&[("k1", k1), ("k2", k2)])).unwrap();
    assert_eq!(verify(&g, &assertion("k2", k2)).await, StatusCode::OK);
    assert_ne!(g.access.last_forced_refetch_ms.load(Ordering::Relaxed), 0);
    // The keys are cached now: the next one needs no fetch (the throttle is still closed).
    assert_eq!(verify(&g, &assertion("k2", k2)).await, StatusCode::OK);
}

#[tokio::test]
async fn unknown_key_ids_earn_a_fetch_only_once_a_minute() {
    let g = gate();
    let [k1, k2, k3] = keys();
    // First unknown key id: fetched, and k2 is still not among the team's keys -> refused.
    assert_eq!(
        verify(&g, &assertion("k2", k2)).await,
        StatusCode::UNAUTHORIZED
    );
    let first = g.access.last_forced_refetch_ms.load(Ordering::Relaxed);
    assert_ne!(first, 0, "the unknown key id caused a fetch");

    // The team publishes k3, but the gate has just fetched: it does not fetch again yet.
    std::fs::write(&g.certs, jwks_json(&[("k1", k1), ("k3", k3)])).unwrap();
    assert_eq!(
        verify(&g, &assertion("k3", k3)).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        g.access.last_forced_refetch_ms.load(Ordering::Relaxed),
        first
    );

    // A minute later it may, and then k3 is accepted.
    g.access.last_forced_refetch_ms.store(1, Ordering::Relaxed);
    assert_eq!(verify(&g, &assertion("k3", k3)).await, StatusCode::OK);
}

#[tokio::test]
async fn a_forged_assertion_never_gets_in_whatever_the_fetch_returns() {
    let g = gate();
    let [k1, k2, _] = keys();
    // Signed by k2 but claiming to be k1: the signature does not verify against k1.
    assert_eq!(
        verify(&g, &assertion("k1", k2)).await,
        StatusCode::UNAUTHORIZED
    );
    // The team's certs name k2 under the id k1's holder never had: still no.
    std::fs::write(&g.certs, jwks_json(&[("k1", k1)])).unwrap();
    assert_eq!(verify(&g, "not.a.jwt").await, StatusCode::UNAUTHORIZED);
    // An unreachable certs URL leaves the gate closed, not open.
    std::fs::remove_file(&g.certs).unwrap();
    assert_eq!(
        verify(&g, &assertion("k9", k2)).await,
        StatusCode::UNAUTHORIZED
    );
}
