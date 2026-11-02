//! Shared handler state and its construction.
//!
//! The sync key is resolved from the vault passphrase file; its disappearance is
//! the authoritative "vault locked" signal (same contract as getSyncKey() in the
//! TS server). Paths and the destruct trigger are injected through `AppConfig` so
//! the whole surface runs against a temp dir with a fake trigger in tests.

use std::sync::{Arc, Mutex};

use sdw_core::sync::crypto::derive_key;
use sdw_core::sync::layout::{SHARED_DIR, SYNC_PASSPHRASE_PATH, UPLOAD_TRACKING_DIR};
use sdw_core::sync::rate_limit::{RateLimitConfig, RateLimiter};
use sdw_core::sync::replay::ReplayWindow;
use sdw_core::vault::destruct::{
    DESTRUCT_KEY_HASH_PATH, DESTRUCT_RATE_LIMIT_GLOBAL_MAX, DESTRUCT_RATE_LIMIT_MAX_ATTEMPTS,
    DESTRUCT_RATE_LIMIT_WINDOW_MS,
};

use sdw_core::auth::session::{derive_session_key_from_password, DEFAULT_SESSION_TTL_SECS};

use crate::destruct::{DestructTrigger, SpawnSelfDestruct};
use crate::features::Features;
use crate::store::Dirs;

/// State for the unified auth gateway: the session-cookie signing key and the credential to check
/// logins against. See deploy/docs/design-unified-auth-gateway.md.
#[derive(Clone)]
pub struct AuthState {
    /// HMAC key for session cookies: 32 random bytes per boot by default (sessions end on
    /// restart), or derived from the master password when AUTH_SESSION_PERSIST is set.
    pub session_key: Arc<[u8; 32]>,
    /// bcrypt hash of the master password (Caddy's DESKTOP_BCRYPT_HASH). None → no password can
    /// authenticate and every login is rejected (fail-closed).
    pub password_hash: Option<Arc<str>>,
    pub ttl_secs: u64,
}

impl AuthState {
    /// Production wiring from the environment (inherited through supervisord from the entrypoint).
    pub fn from_env() -> Self {
        let persist = is_on(std::env::var("AUTH_SESSION_PERSIST").ok().as_deref());
        let password = std::env::var("PASSWORD").ok().filter(|s| !s.is_empty());
        let session_key = match (persist, password.as_deref()) {
            (true, Some(p)) => derive_session_key_from_password(p),
            (true, None) => {
                eprintln!(
                    "auth: AUTH_SESSION_PERSIST is set but PASSWORD is empty; using a random \
                     per-boot key (a generated password changes each boot anyway)"
                );
                random_key()
            }
            _ => random_key(),
        };
        let password_hash = std::env::var("DESKTOP_BCRYPT_HASH")
            .ok()
            .filter(|s| !s.is_empty())
            .map(Arc::from);
        if password_hash.is_none() {
            eprintln!("auth: DESKTOP_BCRYPT_HASH is empty; the login gate will reject all logins");
        }
        Self {
            session_key: Arc::new(session_key),
            password_hash,
            ttl_secs: DEFAULT_SESSION_TTL_SECS,
        }
    }
}

fn random_key() -> [u8; 32] {
    let mut k = [0u8; 32];
    getrandom::getrandom(&mut k).expect("OS RNG unavailable");
    k
}

/// The truthy set the rest of the workspace uses (1/true/yes/on, case-insensitive).
fn is_on(value: Option<&str>) -> bool {
    matches!(
        value.map(|s| s.trim().to_ascii_lowercase()).as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

#[derive(Clone)]
pub struct AppState {
    pub replay: Arc<Mutex<ReplayWindow>>,
    pub limiter: Arc<Mutex<RateLimiter>>,
    pub features: Features,
    pub dirs: Dirs,
    pub passphrase_path: Arc<str>,
    pub key_hash_path: Arc<str>,
    pub destruct: Arc<dyn DestructTrigger>,
    pub auth: AuthState,
    key: Arc<Mutex<Option<[u8; 32]>>>,
}

/// Everything the service needs from the outside world.
pub struct AppConfig {
    pub features: Features,
    pub dirs: Dirs,
    pub passphrase_path: String,
    pub key_hash_path: String,
    pub destruct: Arc<dyn DestructTrigger>,
    pub auth: AuthState,
}

impl AppConfig {
    /// Production wiring: features from the environment, the real container paths,
    /// and the trigger that launches the self-destruct CLI.
    pub fn from_env() -> Self {
        Self {
            features: Features::from_env(),
            dirs: Dirs {
                shared: SHARED_DIR.into(),
                upload: UPLOAD_TRACKING_DIR.into(),
            },
            passphrase_path: SYNC_PASSPHRASE_PATH.into(),
            key_hash_path: DESTRUCT_KEY_HASH_PATH.into(),
            destruct: Arc::new(SpawnSelfDestruct),
            auth: AuthState::from_env(),
        }
    }
}

impl AppState {
    pub fn from_config(cfg: AppConfig) -> Self {
        Self {
            replay: Arc::new(Mutex::new(ReplayWindow::new())),
            limiter: Arc::new(Mutex::new(RateLimiter::new(RateLimitConfig {
                window_ms: DESTRUCT_RATE_LIMIT_WINDOW_MS,
                per_key_max: DESTRUCT_RATE_LIMIT_MAX_ATTEMPTS,
                global_max: DESTRUCT_RATE_LIMIT_GLOBAL_MAX,
            }))),
            features: cfg.features,
            dirs: cfg.dirs,
            passphrase_path: cfg.passphrase_path.into(),
            key_hash_path: cfg.key_hash_path.into(),
            destruct: cfg.destruct,
            auth: cfg.auth,
            key: Arc::new(Mutex::new(None)),
        }
    }

    /// Resolve the AEAD key, honouring vault lock state. A missing passphrase file
    /// drops any cached key and returns None (vault locked).
    pub fn sync_key(&self) -> Option<[u8; 32]> {
        if !std::path::Path::new(&*self.passphrase_path).exists() {
            *self.key.lock().unwrap() = None;
            return None;
        }
        let mut guard = self.key.lock().unwrap();
        if let Some(k) = *guard {
            return Some(k);
        }
        let pass = std::fs::read_to_string(&*self.passphrase_path).ok()?;
        let k = derive_key(pass.trim());
        *guard = Some(k);
        Some(k)
    }

    /// Whether the sync key file is present right now (what `doctor` reports).
    pub fn sync_key_available(&self) -> bool {
        std::path::Path::new(&*self.passphrase_path).exists()
    }
}
