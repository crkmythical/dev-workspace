//! Sync wire-protocol constants shared by every Rust adapter.
//!
//! `sync::client` constructs the browser/native requests and sync-service parses them. Frozen
//! vectors in sync-service/tests/parity.rs preserve compatibility with earlier releases, while
//! system smoke exercises the active HTTP contract end to end. Filesystem paths live in
//! `super::layout`; self-destruct tuning lives in `crate::vault::destruct`.

/// 33-byte PNG header prepended to every encrypted payload (DLP camouflage).
pub const PNG_HEADER: [u8; 33] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, // PNG signature
    0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, // IHDR length + "IHDR"
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, // width=1, height=1
    0x08, 0x02, 0x00, 0x00, 0x00, // bitDepth/colorType/comp/filter/interlace
    0x90, 0x77, 0x53, 0xde, // CRC
];
pub const PNG_HEADER_SIZE: usize = 33;

pub const NONCE_LENGTH: usize = 12;
pub const TAG_LENGTH: usize = 16;
pub const AES_KEY_LENGTH: usize = 32;

// PBKDF2-HMAC-SHA256 over the vault passphrase (see crypto.rs and client.rs).
pub const SYNC_KDF_SALT: &str = "sync-key-v2";
pub const SYNC_KDF_ITERATIONS: u32 = 600_000;

pub const REPLAY_WINDOW_SIZE: usize = 10_000;
pub const REPLAY_TOLERANCE_MS: i64 = 300_000;

// The browser sends one AEAD chunk per request, at most 4 MiB plus framing. Axum's default Bytes
// limit is 2 MiB, so the app raises it to a bounded 5 MiB.
pub const MAX_REQUEST_BODY_BYTES: usize = 5 * 1024 * 1024;

// Both clients enforce these, and the server independently caps assembled bytes and chunk count.
pub const CHUNK_SIZE_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_UPLOAD_BYTES: u64 = 500 * 1024 * 1024;
pub const MAX_UPLOAD_CHUNKS: u64 = MAX_UPLOAD_BYTES.div_ceil(CHUNK_SIZE_BYTES);

pub const HEADER_OP: &str = "x-sync-op";
pub const HEADER_TS: &str = "x-sync-ts";
pub const HEADER_FILE_ID: &str = "x-sync-fileid";
pub const HEADER_CHUNK_IDX: &str = "x-sync-chunkidx";
pub const HEADER_AUTH: &str = "x-sync-auth";

pub const API_UPLOAD: &str = "/sync/api/upload";
pub const API_DOWNLOAD: &str = "/sync/api/download";
pub const API_DOCTOR: &str = "/sync/api/doctor";
pub const API_DESTRUCT: &str = "/sync/api/destruct";
/// Prometheus text metrics (METRICS_ENABLED=1). Under /sync/api/ so Caddy's gate covers it.
pub const API_METRICS: &str = "/sync/api/metrics";

#[cfg(test)]
mod tests {
    use super::*;

    /// Freeze the on-the-wire contract used by the Rust server, WASM and native client.
    #[test]
    fn wire_contract_is_frozen() {
        assert_eq!(HEADER_OP, "x-sync-op");
        assert_eq!(HEADER_TS, "x-sync-ts");
        assert_eq!(HEADER_FILE_ID, "x-sync-fileid");
        assert_eq!(HEADER_CHUNK_IDX, "x-sync-chunkidx");
        assert_eq!(HEADER_AUTH, "x-sync-auth");
        assert_eq!(API_UPLOAD, "/sync/api/upload");
        assert_eq!(API_DOWNLOAD, "/sync/api/download");
        assert_eq!(API_DOCTOR, "/sync/api/doctor");
        assert_eq!(API_DESTRUCT, "/sync/api/destruct");
        assert_eq!(SYNC_KDF_SALT, "sync-key-v2");
        assert_eq!(SYNC_KDF_ITERATIONS, 600_000);
        assert_eq!((NONCE_LENGTH, TAG_LENGTH, AES_KEY_LENGTH), (12, 16, 32));
        assert_eq!((REPLAY_WINDOW_SIZE, REPLAY_TOLERANCE_MS), (10_000, 300_000));
    }
}
