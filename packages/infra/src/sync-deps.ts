/** The production wiring of the sync use cases' ports. */
import { ReplayWindow, type SyncDeps } from "@sdw/core/sync";
import { nodeAead } from "./sync-crypto.ts";
import { createFileKeyProvider } from "./sync-key.ts";
import { createFsUploadStore } from "./sync-store.ts";

export function createSyncDeps(): SyncDeps {
  return {
    keys: createFileKeyProvider(),
    aead: nodeAead,
    store: createFsUploadStore(),
    replay: new ReplayWindow(),
  };
}
