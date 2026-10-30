//! Mirrors the TS layout/protocol constants the sync-service needs
//! (packages/core/src/sync/{protocol,layout}.ts, vault/destruct.ts,
//! workspace/features.ts). These MUST stay in step with the TS side — the wire
//! vectors pin the protocol ones; the HTTP contract (tests/smoke) pins the rest.

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

/// The image build writes the compiled SPA here; the daemon serves it under /sync/
/// (Caddy proxies all of /sync/* to this port).
pub const SPA_DIST_DIR: &str = "/opt/workspace/packages/spa/dist";

pub const REPLAY_WINDOW_SIZE: usize = 10_000;
pub const REPLAY_TOLERANCE_MS: i64 = 300_000;

// Remote self-destruct (packages/core/src/vault/destruct.ts).
pub const DESTRUCT_KEY_HASH_PATH: &str = "/var/run/self-destruct/key-hash";
pub const DESTRUCT_RATE_LIMIT_WINDOW_MS: i64 = 60_000;
pub const DESTRUCT_RATE_LIMIT_MAX_ATTEMPTS: u64 = 3;
/// Absolute cap across ALL clients per window: bounds a brute force that rotates
/// the client-influenced IP header to defeat the per-key budget.
pub const DESTRUCT_RATE_LIMIT_GLOBAL_MAX: u64 = 10;
pub const DESTRUCT_RESPONSE_DELAY_MS: u64 = 200;
