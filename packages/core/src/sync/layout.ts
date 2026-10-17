/** Where the sync service lives and keeps its files. */
import { CREDENTIALS_DIR, WORKSPACE_MOUNT } from "../shared/layout.ts";

export const SYNC_SERVICE_PORT = 8081;

/** Where the image build puts the compiled SPA (served by the sync service under /sync/). */
export const SPA_DIST_DIR = "/opt/workspace/packages/spa/dist";

/** Files received from the browser end up here. */
export const SHARED_DIR = `${WORKSPACE_MOUNT}/shared`;

/** In-flight chunked uploads (stale ones are cleaned by image/etc/cron.d/cleanup). */
export const UPLOAD_TRACKING_DIR = `${SHARED_DIR}/.uploads`;

/**
 * The AEAD sync key is derived from this file. It lives inside the vault, so it
 * is only readable while the vault is mounted: its disappearance is the
 * authoritative "vault locked" signal.
 */
export const SYNC_PASSPHRASE_PATH = `${CREDENTIALS_DIR}/sync-passphrase`;
