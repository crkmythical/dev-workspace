//! Shared handler state and its construction.
//!
//! The sync key is resolved from the vault passphrase file; its disappearance is
//! the authoritative "vault locked" signal (same contract as getSyncKey() in the
//! TS server). Paths and the destruct trigger are injected through `AppConfig` so
//! the whole surface runs against a temp dir with a fake trigger in tests.

use std::sync::{Arc, Mutex};

use crate::constants::{
    DESTRUCT_KEY_HASH_PATH, DESTRUCT_RATE_LIMIT_GLOBAL_MAX, DESTRUCT_RATE_LIMIT_MAX_ATTEMPTS,
    DESTRUCT_RATE_LIMIT_WINDOW_MS, SHARED_DIR, SPA_DIST_DIR, SYNC_PASSPHRASE_PATH,
    UPLOAD_TRACKING_DIR,
};
use crate::crypto::derive_key;
use crate::destruct::{DestructTrigger, SpawnSelfDestruct};
use crate::features::Features;
use crate::rate_limit::{RateLimitConfig, RateLimiter};
use crate::replay::ReplayWindow;
use crate::store::Dirs;

#[derive(Clone)]
pub struct AppState {
    pub replay: Arc<Mutex<ReplayWindow>>,
    pub limiter: Arc<Mutex<RateLimiter>>,
    pub features: Features,
    pub dirs: Dirs,
    pub passphrase_path: Arc<str>,
    pub key_hash_path: Arc<str>,
    pub spa_dir: Arc<str>,
    pub destruct: Arc<dyn DestructTrigger>,
    key: Arc<Mutex<Option<[u8; 32]>>>,
}

/// Everything the service needs from the outside world.
pub struct AppConfig {
    pub features: Features,
    pub dirs: Dirs,
    pub passphrase_path: String,
    pub key_hash_path: String,
    pub spa_dir: String,
    pub destruct: Arc<dyn DestructTrigger>,
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
            spa_dir: SPA_DIST_DIR.into(),
            destruct: Arc::new(SpawnSelfDestruct),
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
            spa_dir: cfg.spa_dir.into(),
            destruct: cfg.destruct,
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
