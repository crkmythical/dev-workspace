//! Sync wire-protocol constants — a port of packages/core/src/sync/protocol.ts.
//!
//! These pin the on-the-wire envelope and MUST stay in step with any other
//! implementation: sync-service/tests/parity.rs checks them against golden
//! vectors generated from the authoritative TS side, and tests/smoke exercises
//! the HTTP contract end to end. The layout/path constants live in
//! `super::layout`; the self-destruct tuning in `crate::vault::destruct`.

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

// Sync key derivation: PBKDF2-HMAC-SHA256 over the vault passphrase (see crypto.rs + client.ts).
pub const SYNC_KDF_SALT: &str = "sync-key-v2";
pub const SYNC_KDF_ITERATIONS: u32 = 600_000;

// Replay-window policy (super::replay).
pub const REPLAY_WINDOW_SIZE: usize = 10_000;
pub const REPLAY_TOLERANCE_MS: i64 = 300_000;

// Max request body the API accepts. The browser sends one AEAD chunk per request, up to
// CHUNK_SIZE = 4 MiB (packages/core/src/sync/protocol.ts) plus the 33-byte PNG header, 12-byte
// nonce and 16-byte tag. axum's default `Bytes` limit is only 2 MiB, which would 413 a real chunk
// before the handler (so files over ~2 MB could never upload); app() raises the limit to this. It
// also bounds how much a single request can buffer in memory.
pub const MAX_REQUEST_BODY_BYTES: usize = 5 * 1024 * 1024;

// Server-side ceiling on one assembled upload, mirroring the browser client's SIZE_LIMIT
// (packages/core/src/sync/protocol.ts = 500 MB). The client enforces it too, but the server must
// not trust the client: without this an authenticated client (or a looping one) could grow a
// single upload without bound and fill the vault disk. append_chunk refuses a crossing chunk.
pub const MAX_UPLOAD_BYTES: u64 = 500 * 1024 * 1024;

// HTTP wire contract: header names + API paths, the single Rust source the routes use. Mirrors
// SYNC_HEADERS / SYNC_API in packages/core/src/sync/protocol.ts — the frozen values the browser
// SPA speaks. The test below pins them so a Rust-side typo is caught; tests/smoke checks the two
// languages agree end to end.
pub const HEADER_OP: &str = "x-sync-op";
pub const HEADER_TS: &str = "x-sync-ts";
pub const HEADER_FILE_ID: &str = "x-sync-fileid";
pub const HEADER_CHUNK_IDX: &str = "x-sync-chunkidx";
pub const HEADER_AUTH: &str = "x-sync-auth";

pub const API_UPLOAD: &str = "/sync/api/upload";
pub const API_DOWNLOAD: &str = "/sync/api/download";
pub const API_DOCTOR: &str = "/sync/api/doctor";
pub const API_DESTRUCT: &str = "/sync/api/destruct";

#[cfg(test)]
mod tests {
    use super::*;

    /// Freeze the on-the-wire contract. Changing any of these breaks the browser SPA, so a change
    /// here must be deliberate and mirrored in the TS client (+ the smoke suite).
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
