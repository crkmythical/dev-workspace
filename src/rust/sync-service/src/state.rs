//! Shared handler state and its construction.
//!
//! The sync key is resolved from the vault passphrase file; its disappearance is
//! the authoritative "vault locked" signal (same contract as getSyncKey() in the
//! TS server). Paths and the destruct trigger are injected through `AppConfig` so
//! the whole surface runs against a temp dir with a fake trigger in tests.

use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use sdw_core::sync::crypto::derive_key;
use sdw_core::sync::layout::{SHARED_DIR, SYNC_PASSPHRASE_PATH, UPLOAD_TRACKING_DIR};
use sdw_core::sync::rate_limit::{RateLimitConfig, RateLimiter};
use sdw_core::sync::replay::ReplayWindow;
use sdw_core::vault::destruct::{
    DESTRUCT_KEY_HASH_PATH, DESTRUCT_RATE_LIMIT_GLOBAL_MAX, DESTRUCT_RATE_LIMIT_MAX_ATTEMPTS,
    DESTRUCT_RATE_LIMIT_WINDOW_MS,
};

use sdw_core::audit::{AuditSink, AUDIT_LOG_PATH};
use sdw_core::auth::access_jwt::Jwks;
use sdw_core::auth::session::{
    DEFAULT_SESSION_TTL_SECS, LOGIN_RATE_LIMIT_GLOBAL, LOGIN_RATE_LIMIT_PER_KEY,
    LOGIN_RATE_LIMIT_WINDOW_MS, SESSION_KEY_PATH,
};

use crate::audit::{Auditor, Metrics};
use crate::destruct::{DestructTrigger, SpawnSelfDestruct};
use crate::features::Features;
use crate::store::Dirs;

/// State for the unified auth gateway: the session-cookie signing key and the credential to check
/// logins against. See deploy/docs/design-unified-auth-gateway.md.
#[derive(Clone)]
pub struct AuthState {
    /// HMAC key for session cookies: 32 random bytes per boot by default (sessions end on
    /// restart), or a random key kept in SESSION_KEY_PATH when AUTH_SESSION_PERSIST is set.
    pub session_key: Arc<[u8; 32]>,
    /// bcrypt hash of the master password (Caddy's DESKTOP_BCRYPT_HASH). None → no password can
    /// authenticate and every login is rejected (fail-closed).
    pub password_hash: Option<Arc<str>>,
    pub ttl_secs: u64,
    /// Cloudflare Access acceptance, when the deployment sits behind Access and names its team +
    /// app. None → the gate is cookie-only (a deployment not behind Access never trusts the
    /// Cf-Access-Jwt-Assertion header). See deploy/docs/design-unified-auth-gateway.md §7.
    pub access: Option<AccessState>,
}

/// What the gate needs to accept a Cloudflare Access assertion: the app AUD, the team issuer, the
/// certs URL, and the JWKS cached from it (refreshed in the background by the sync-service).
#[derive(Clone)]
pub struct AccessState {
    pub aud: Arc<str>,
    pub issuer: Arc<str>,
    pub certs_url: Arc<str>,
    pub jwks: Arc<RwLock<Jwks>>,
    /// When the keys were last fetched because an assertion named a key we do not have (ms since
    /// the epoch; 0 = never). See [`AccessState::claim_refetch`].
    pub last_forced_refetch_ms: Arc<AtomicI64>,
}

/// An assertion signed by a key the cached JWKS lacks is the normal sign of Cloudflare rotating its
/// signing keys. It earns one fetch, at most this often, so a stream of forged key ids cannot make
/// the gate fetch without limit.
pub const FORCED_REFETCH_MIN_INTERVAL_MS: i64 = 60_000;

impl AccessState {
    /// Whether the caller may fetch the keys now because of an unknown key id. True for at most one
    /// caller per [`FORCED_REFETCH_MIN_INTERVAL_MS`]; the others go on with the keys they have.
    pub fn claim_refetch(&self, now_ms: i64) -> bool {
        let last = self.last_forced_refetch_ms.load(Ordering::Relaxed);
        now_ms.saturating_sub(last) >= FORCED_REFETCH_MIN_INTERVAL_MS
            && self
                .last_forced_refetch_ms
                .compare_exchange(last, now_ms, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
    }
}

impl AuthState {
    /// Production wiring from the environment (inherited through supervisord from the entrypoint).
    pub fn from_env() -> Self {
        let persist = is_on(std::env::var("AUTH_SESSION_PERSIST").ok().as_deref());
        // Sessions survive a restart only when asked, and then through a random key on disk: a key
        // derived from the password would let a seen cookie be used to guess the password offline.
        let session_key = if persist {
            sdw_infra::secrets::persistent_key(std::path::Path::new(SESSION_KEY_PATH))
                .unwrap_or_else(|e| {
                    eprintln!("auth: cannot keep a session key ({e}); using a random per-boot key");
                    random_key()
                })
        } else {
            random_key()
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
            access: access_from_env(),
        }
    }
}

/// Build the Access acceptance config from the environment, or None when not behind Access. Both
/// ACCESS_TEAM_DOMAIN and ACCESS_AUD must be set; otherwise the header is never trusted.
fn access_from_env() -> Option<AccessState> {
    let var = |k: &str| {
        std::env::var(k)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let domain = var("ACCESS_TEAM_DOMAIN")?;
    let aud = var("ACCESS_AUD")?;
    let domain = domain.trim_end_matches('/');
    Some(AccessState {
        aud: Arc::from(aud),
        issuer: Arc::from(format!("https://{domain}")),
        certs_url: Arc::from(format!("https://{domain}/cdn-cgi/access/certs")),
        jwks: Arc::new(RwLock::new(Jwks::default())),
        last_forced_refetch_ms: Arc::new(AtomicI64::new(0)),
    })
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

const MAX_TRACKED_UPLOADS: usize = 2_048;
const UPLOAD_SESSION_STALE_MS: i64 = 6 * 60 * 60 * 1_000;

/// Per-file upload sequencing. Requests for different file ids remain concurrent; requests for the
/// same id are serialized so append-size checks, chunk order, checksum and rename form one logical
/// transaction.
#[derive(Default)]
pub struct UploadSession {
    pub next_chunk: u64,
    pub finalized: bool,
    pub last_seen_ms: i64,
}

pub(crate) type UploadLock = Arc<tokio::sync::Mutex<UploadSession>>;
type UploadMap = Arc<tokio::sync::Mutex<HashMap<String, UploadLock>>>;

#[derive(Clone, Copy, PartialEq, Eq)]
struct KeyStamp {
    device: u64,
    inode: u64,
    size: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
}

impl From<&std::fs::Metadata> for KeyStamp {
    fn from(metadata: &std::fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.size(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
        }
    }
}

struct CachedSyncKey {
    stamp: KeyStamp,
    key: [u8; 32],
}

impl Drop for CachedSyncKey {
    fn drop(&mut self) {
        self.key.fill(0);
    }
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
    /// Idempotence guard: only one irreversible destroy process may be launched at a time.
    pub destruct_started: Arc<AtomicBool>,
    pub auth: AuthState,
    /// Throttle for `POST /auth/login` (separate from the self-destruct limiter).
    pub login_limiter: Arc<Mutex<RateLimiter>>,
    pub auditor: Arc<Auditor>,
    pub metrics: Arc<Metrics>,
    /// `GET /sync/api/metrics` answers only when METRICS_ENABLED is on (else 404).
    pub metrics_enabled: bool,
    key: Arc<Mutex<Option<CachedSyncKey>>>,
    uploads: UploadMap,
}

/// Everything the service needs from the outside world.
pub struct AppConfig {
    pub features: Features,
    pub dirs: Dirs,
    pub passphrase_path: String,
    pub key_hash_path: String,
    pub destruct: Arc<dyn DestructTrigger>,
    pub auth: AuthState,
    /// Where audit events go (the file under /var/log/sdw in production, memory in tests).
    pub audit: Arc<dyn AuditSink>,
    pub metrics_enabled: bool,
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
            // Also on stdout, so `docker logs` holds a copy outside the container's file system.
            audit: Arc::new(sdw_infra::audit::FileAuditSink::new(AUDIT_LOG_PATH).echo_stdout(true)),
            metrics_enabled: is_on(std::env::var("METRICS_ENABLED").ok().as_deref()),
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
            destruct_started: Arc::new(AtomicBool::new(false)),
            auth: cfg.auth,
            login_limiter: Arc::new(Mutex::new(RateLimiter::new(RateLimitConfig {
                window_ms: LOGIN_RATE_LIMIT_WINDOW_MS,
                per_key_max: LOGIN_RATE_LIMIT_PER_KEY,
                global_max: LOGIN_RATE_LIMIT_GLOBAL,
            }))),
            auditor: Arc::new(Auditor::new(cfg.audit)),
            metrics: Arc::new(Metrics::default()),
            metrics_enabled: cfg.metrics_enabled,
            key: Arc::new(Mutex::new(None)),
            uploads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    /// Return the sequencer for one upload id. The registry is bounded; completed and stale idle
    /// entries are discarded before admitting more, so an authenticated client cannot grow server
    /// memory without limit by minting ids and abandoning them.
    pub(crate) async fn upload_session(&self, id: &str, now_ms: i64) -> Option<UploadLock> {
        let mut uploads = self.uploads.lock().await;
        if let Some(session) = uploads.get(id) {
            return Some(session.clone());
        }
        if uploads.len() >= MAX_TRACKED_UPLOADS {
            uploads.retain(|_, session| {
                session.try_lock().map_or(true, |state| {
                    !state.finalized
                        && now_ms.saturating_sub(state.last_seen_ms) < UPLOAD_SESSION_STALE_MS
                })
            });
        }
        if uploads.len() >= MAX_TRACKED_UPLOADS {
            return None;
        }
        let session = Arc::new(tokio::sync::Mutex::new(UploadSession {
            last_seen_ms: now_ms,
            ..UploadSession::default()
        }));
        uploads.insert(id.to_string(), session.clone());
        Some(session)
    }

    /// Resolve the AEAD key while honouring lock and password-rotation state. The passphrase file
    /// is atomically replaced on unlock; device/inode/mtime/size therefore form a cheap cache key.
    /// A missing or unreadable file immediately drops and zeroes the cached key.
    pub fn sync_key(&self) -> Option<[u8; 32]> {
        let metadata = match std::fs::metadata(&*self.passphrase_path) {
            Ok(metadata) => metadata,
            Err(_) => {
                *self.key.lock().unwrap() = None;
                return None;
            }
        };
        let stamp = KeyStamp::from(&metadata);
        let mut guard = self.key.lock().unwrap();
        if let Some(cached) = guard.as_ref().filter(|cached| cached.stamp == stamp) {
            return Some(cached.key);
        }
        let pass = match std::fs::read_to_string(&*self.passphrase_path) {
            Ok(pass) => pass,
            Err(_) => {
                *guard = None;
                return None;
            }
        };
        let key = derive_key(pass.trim());
        *guard = Some(CachedSyncKey { stamp, key });
        Some(key)
    }

    /// Whether the sync key file is present right now (what `doctor` reports).
    pub fn sync_key_available(&self) -> bool {
        std::path::Path::new(&*self.passphrase_path).exists()
    }
}
