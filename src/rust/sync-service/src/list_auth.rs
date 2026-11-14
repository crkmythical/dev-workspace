//! Authenticator for listing the shared directory, paired with
//! `sdw_core::sync::client::build_list_headers`.
//!
//! The client sends `X-Sync-Ts` and `X-Sync-Auth` = base64(nonce || tag): an
//! AES-GCM envelope over an EMPTY message whose AAD is `list|<ts>`. Only a holder
//! of the sync key can produce one.
//!
//! Order matters: the nonce is claimed in the replay window only AFTER the tag
//! verifies. The window is a bounded ring buffer, so if unauthenticated requests
//! could claim slots, a flood of forgeries would push a genuine nonce out and let a
//! captured request be replayed.
use std::sync::Mutex;

use base64::{engine::general_purpose::STANDARD, Engine as _};

use sdw_core::sync::aad::build_aad;
use sdw_core::sync::crypto::decrypt;
use sdw_core::sync::protocol::{NONCE_LENGTH, TAG_LENGTH};
use sdw_core::sync::replay::ReplayWindow;

/// Why a list request was refused. The HTTP layer answers every variant with the
/// same 401, so a caller learns nothing about which check failed.
#[derive(Debug, PartialEq, Eq)]
pub enum ListAuthError {
    Malformed,
    BadTag,
    Replay,
}

pub fn authenticate_list(
    key: &[u8],
    ts: i64,
    auth_b64: &str,
    replay: &Mutex<ReplayWindow>,
    now_ms: i64,
) -> Result<(), ListAuthError> {
    let bytes = STANDARD
        .decode(auth_b64)
        .map_err(|_| ListAuthError::Malformed)?;
    // nonce || tag, with nothing between them: the message is empty.
    if bytes.len() != NONCE_LENGTH + TAG_LENGTH {
        return Err(ListAuthError::Malformed);
    }
    let (nonce, tag) = bytes.split_at(NONCE_LENGTH);

    decrypt(key, nonce, tag, &build_aad("list", ts, &[])).map_err(|_| ListAuthError::BadTag)?;

    let claimed =
        replay
            .lock()
            .expect("replay window lock poisoned")
            .check(&hex::encode(nonce), ts, now_ms);
    if claimed {
        Ok(())
    } else {
        Err(ListAuthError::Replay)
    }
}
