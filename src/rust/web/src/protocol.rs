use sdw_core::sync::client::{
    build_chunk_request, build_finalize_request, build_list_headers, SyncRequest as CoreRequest,
};
use sdw_core::sync::crypto::derive_key;
use sdw_core::sync::protocol::{CHUNK_SIZE_BYTES, MAX_UPLOAD_BYTES, NONCE_LENGTH};

use crate::browser::{now_ms, random_bytes};

pub const API_UPLOAD: &str = "/sync/api/upload";
pub const API_LIST: &str = "/sync/api/download";
pub const API_DOCTOR: &str = "/sync/api/doctor";
pub const API_DESTRUCT: &str = "/sync/api/destruct";
pub const CHUNK_SIZE: u64 = CHUNK_SIZE_BYTES;
pub const SIZE_LIMIT: u64 = MAX_UPLOAD_BYTES;
pub const POLL_FOCUSED_MS: u32 = 2_000;
pub const POLL_BACKGROUND_MS: u32 = 10_000;

pub type SyncKey = [u8; 32];

pub struct SyncRequest {
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl From<CoreRequest> for SyncRequest {
    fn from(request: CoreRequest) -> Self {
        Self {
            headers: request
                .headers
                .into_iter()
                .map(|(name, value)| (name.to_string(), value))
                .collect(),
            body: request.body,
        }
    }
}

pub fn derive_sync_key(passphrase: &str) -> SyncKey {
    derive_key(passphrase)
}

pub fn chunk_request(
    key: &SyncKey,
    file_id: &str,
    chunk_index: u64,
    chunk: &[u8],
) -> Result<SyncRequest, String> {
    build_chunk_request(
        key,
        now_ms(),
        file_id,
        chunk_index,
        chunk,
        random_bytes::<NONCE_LENGTH>()?,
    )
    .map(Into::into)
}

pub fn finalize_request(
    key: &SyncKey,
    file_id: &str,
    file_path: &str,
    content_sha256: &str,
    total_chunks: u64,
) -> Result<SyncRequest, String> {
    build_finalize_request(
        key,
        now_ms(),
        file_id,
        file_path,
        content_sha256,
        total_chunks,
        random_bytes::<NONCE_LENGTH>()?,
    )
    .map(Into::into)
}

pub fn list_headers(key: &SyncKey) -> Result<Vec<(String, String)>, String> {
    build_list_headers(key, now_ms(), random_bytes::<NONCE_LENGTH>()?).map(|headers| {
        headers
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect()
    })
}

pub fn random_file_id() -> Result<String, String> {
    Ok(random_bytes::<16>()?
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
