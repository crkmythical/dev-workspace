/** Self-destruct: state paths, key derivation parameters and the remote trigger's rate limit. */
import { RUN_DIR } from "../shared/layout.ts";

/** Persisted on a named volume so it survives container recreation. */
export const DESTRUCT_STATE_DIR = `${RUN_DIR}/self-destruct`;
export const DESTRUCT_KEY_HASH_PATH = `${DESTRUCT_STATE_DIR}/key-hash`;
/** Written last by a destroy. Its presence puts the container into inert mode (entrypoint.sh). */
export const DESTRUCT_COMPLETED_PATH = `${DESTRUCT_STATE_DIR}/completed`;

// Key derivation (HKDF-SHA256), domain-separated from the sync key.
export const DESTRUCT_KEY_LENGTH = 32;
export const DESTRUCT_HKDF_SALT = "destruct-key-v1";
export const DESTRUCT_HKDF_INFO = "self-destruct";

// Remote trigger rate limit.
export const DESTRUCT_RATE_LIMIT_WINDOW_MS = 60_000;
export const DESTRUCT_RATE_LIMIT_MAX_ATTEMPTS = 3;
// Global backstop: absolute cap on destruct attempts per window across ALL
// clients. Bounds a brute force that rotates X-Forwarded-For to defeat the
// per-IP budget (the per-IP key is client-influenced; this cap is not).
export const DESTRUCT_RATE_LIMIT_GLOBAL_MAX = 10;
export const DESTRUCT_RESPONSE_DELAY_MS = 200;
