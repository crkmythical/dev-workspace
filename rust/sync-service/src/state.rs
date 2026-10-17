//! Shared handler state. The sync key is resolved from the vault passphrase
//! file; its disappearance is the authoritative "vault locked" signal (same
//! contract as getSyncKey() in packages/server/src/routes/upload.ts).

use std::sync::{Arc, Mutex};

use crate::constants::SYNC_PASSPHRASE_PATH;
use crate::crypto::derive_key;
use crate::replay::ReplayWindow;

#[derive(Clone)]
pub struct AppState {
    pub replay: Arc<Mutex<ReplayWindow>>,
    key: Arc<Mutex<Option<[u8; 32]>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            replay: Arc::new(Mutex::new(ReplayWindow::new())),
            key: Arc::new(Mutex::new(None)),
        }
    }

    /// Resolve the AEAD key, honouring vault lock state. Missing passphrase file
    /// → drop any cached key and return None (vault locked).
    pub fn sync_key(&self) -> Option<[u8; 32]> {
        if !std::path::Path::new(SYNC_PASSPHRASE_PATH).exists() {
            *self.key.lock().unwrap() = None;
            return None;
        }
        let mut guard = self.key.lock().unwrap();
        if let Some(k) = *guard {
            return Some(k);
        }
        let pass = std::fs::read_to_string(SYNC_PASSPHRASE_PATH).ok()?;
        let k = derive_key(pass.trim());
        *guard = Some(k);
        Some(k)
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
