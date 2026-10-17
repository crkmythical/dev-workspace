/**
 * Optional features, all OFF unless explicitly enabled in the environment.
 *
 *   SYNC_ENABLED=1             browser file-sync page + its upload/download API.
 *                              Needs the vault passphrase written to a file inside
 *                              the vault on every unlock (the sync key source).
 *   REMOTE_DESTRUCT_ENABLED=1  POST /sync/api/destruct. Needs a derived key hash
 *                              stored on every unlock.
 *
 * A feature that is off removes its stale credential material on the next
 * unlock, so turning a flag off is also a clean-up.
 */
export interface FeatureFlags {
  sync: boolean;
  remoteDestruct: boolean;
}

const TRUTHY = new Set(["1", "true", "yes", "on"]);

function isOn(value: string | undefined): boolean {
  return TRUTHY.has((value ?? "").trim().toLowerCase());
}

export function parseFeatureFlags(env: Record<string, string | undefined>): FeatureFlags {
  return {
    sync: isOn(env.SYNC_ENABLED),
    remoteDestruct: isOn(env.REMOTE_DESTRUCT_ENABLED),
  };
}
