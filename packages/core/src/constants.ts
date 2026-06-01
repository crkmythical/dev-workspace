// Shared constants across all packages

// Ports
export const CLASH_HTTP_PORT = 7890;
export const CLASH_SOCKS_PORT = 7891;
export const CLASH_CONTROLLER_PORT = 9090;
// CADDY_PORT documents the in-container reverse-proxy port. It is referenced by
// the baked Caddyfile and docker-compose port mapping (not by TS), kept here as
// the canonical value so the topology has one source of truth.
export const CADDY_PORT = 8080;
export const SYNC_SERVICE_PORT = 8081;
export const CODE_SERVER_PORT = 8082;

// Paths
export const VAULT_CIPHER_DIR = "/vault/cipher";
export const WORKSPACE_MOUNT = "/workspace";
export const SHARED_DIR = "/workspace/shared";
export const CREDENTIALS_DIR = "/workspace/.credentials";
export const SYNC_PASSPHRASE_PATH = "/workspace/.credentials/sync-passphrase";
export const STATE_SOCKET_PATH = "/var/run/vault-state.sock";
export const VAULT_SYNC_STATE_DIR = "/var/run/vault-sync";
export const UPLOAD_TRACKING_DIR = "/workspace/shared/.uploads";

// Crypto
export const HKDF_SALT = "sync-key-v1";
export const HKDF_INFO = "aead-key";
export const NONCE_LENGTH = 12;
export const TAG_LENGTH = 16;
export const AES_KEY_LENGTH = 32;

// PNG camouflage (33 bytes)
export const PNG_HEADER = new Uint8Array([
  0x89,
  0x50,
  0x4e,
  0x47,
  0x0d,
  0x0a,
  0x1a,
  0x0a, // PNG signature (8)
  0x00,
  0x00,
  0x00,
  0x0d, // IHDR length (4)
  0x49,
  0x48,
  0x44,
  0x52, // "IHDR" (4)
  0x00,
  0x00,
  0x00,
  0x01, // width=1 (4)
  0x00,
  0x00,
  0x00,
  0x01, // height=1 (4)
  0x08,
  0x02,
  0x00,
  0x00,
  0x00, // bitDepth=8, colorType=2, comp=0, filter=0, interlace=0 (5)
  0x90,
  0x77,
  0x53,
  0xde, // CRC (4)
]);
export const PNG_HEADER_SIZE = 33;

// Sync
export const CHUNK_SIZE = 4 * 1024 * 1024; // 4 MiB
export const SIZE_LIMIT = 500 * 1024 * 1024; // 500 MB
export const REPLAY_WINDOW_SIZE = 10000;
export const REPLAY_TOLERANCE_MS = 300_000; // 5 min

// Reserved for not-yet-wired spec features. Kept as the single source of truth
// so the eventual implementation references these instead of fresh magic
// numbers. POLL_INTERVAL_* — SPA adaptive polling (currently hardcoded in
// packages/spa/src/main.ts); STALE_UPLOAD_THRESHOLD_MS — orphan .uploading-*
// cleanup cron (Req 27.5); GRACEFUL_RESTART_INTERVAL_MS — sync-service periodic
// restart (Req 27.1).
export const POLL_INTERVAL_FOCUSED_MS = 2000;
export const POLL_INTERVAL_BACKGROUND_MS = 10000;
export const STALE_UPLOAD_THRESHOLD_MS = 60 * 60 * 1000; // 1h
export const GRACEFUL_RESTART_INTERVAL_MS = 24 * 60 * 60 * 1000; // 24h

// Vault sync
export const DEFAULT_VAULT_SYNC_INTERVAL = 1800; // 30 min
export const SYNC_FAILURE_THRESHOLD = 3;

// Pentest environment (dual vault)
export const PENTEST_CIPHER_DIR = "/pentest/cipher";
export const PENTEST_MOUNT = "/pentest/rootfs";
export const PENTEST_TOR_PORT = 9050;

// Self-destruct
export const DESTRUCT_STATE_DIR = "/var/run/self-destruct";
export const DESTRUCT_KEY_HASH_PATH = "/var/run/self-destruct/key-hash";
export const DESTRUCT_COMPLETED_PATH = "/var/run/self-destruct/completed";
export const DESTRUCT_HKDF_SALT = "destruct-key-v1";
export const DESTRUCT_HKDF_INFO = "self-destruct";
export const DESTRUCT_RATE_LIMIT_WINDOW_MS = 60_000;
export const DESTRUCT_RATE_LIMIT_MAX_ATTEMPTS = 3;
export const DESTRUCT_RESPONSE_DELAY_MS = 200;

// Remote desktop (Selkies stream)
export const DESKTOP_HOME = "/workspace/.desktop";
export const DESKTOP_CONFIG_DIR = "/workspace/.desktop/.config";
export const DESKTOP_CACHE_DIR = "/tmp/.desktop-cache";
export const DESKTOP_APPS_DIR = "/opt/desktop-apps";
export const DESKTOP_APPS_REGISTRY = "/opt/desktop-apps/.registry.json";
/** Port the Selkies WebSocket server listens on (--port=6080). Selkies' own
 *  default is 8082 which collides with code-server; we use 6080 instead. */
export const DESKTOP_STREAM_PORT = 6080;
/** X display the desktop runs on. Xvfb, selkies, and XFCE all bind here. */
export const DESKTOP_DISPLAY = ":1";
/** Path where the static Selkies web client is served from by Caddy. */
export const DESKTOP_WEB_ROOT = "/usr/share/selkies/web";
export const DESKTOP_DEFAULT_RESOLUTION = "1920x1080";

// --- Dual-desktop (selkies + kasmvnc parallel) ---
// Selkies stack: H.264/WebCodecs on display :1, served at /desktop/.
// DESKTOP_STREAM_PORT/DESKTOP_DISPLAY/DESKTOP_HOME above are the selkies values
// (kept as the canonical names for backward-compat with doctor/tests).
export const SELKIES_STREAM_PORT = DESKTOP_STREAM_PORT; // 6080
export const SELKIES_DISPLAY = DESKTOP_DISPLAY; // ":1"
export const SELKIES_HOME = DESKTOP_HOME; // /workspace/.desktop

// KasmVNC stack: integrated X+VNC server on display :2, served at /vnc/.
// Independent HOME so its XFCE session never shares state with selkies'.
export const VNC_STREAM_PORT = 6081;
export const VNC_DISPLAY = ":2";
export const VNC_HOME = "/workspace/.desktop-vnc";
export const VNC_CONFIG_DIR = "/workspace/.desktop-vnc/.config";
/** Path where the static KasmVNC web client is served from (its own .deb). */
export const VNC_WEB_ROOT = "/usr/share/kasmvnc/www";
