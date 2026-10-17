//! Mirrors packages/core/src/constants.ts (the subset the sync-service needs).
//! These MUST stay byte-identical to the TS side — the parity tests enforce it.

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

pub const HKDF_SALT: &str = "sync-key-v1";
pub const HKDF_INFO: &str = "aead-key";

pub const SHARED_DIR: &str = "/workspace/shared";
pub const UPLOAD_TRACKING_DIR: &str = "/workspace/shared/.uploads";
pub const SYNC_PASSPHRASE_PATH: &str = "/workspace/.credentials/sync-passphrase";
pub const SYNC_SERVICE_PORT: u16 = 8081;

pub const REPLAY_WINDOW_SIZE: usize = 10_000;
pub const REPLAY_TOLERANCE_MS: i64 = 300_000;
