/**
 * Deployment defaults for the two vault instances. Only composition roots
 * (cli / server / entrypoint) should import these; they are replaced by an
 * injected Config when constants stop being global.
 */
import {
  PENTEST_CIPHER_DIR,
  PENTEST_MOUNT,
  VAULT_CIPHER_DIR,
  WORKSPACE_MOUNT,
} from "../constants.ts";
import type { VaultSpec } from "./domain.ts";

export const WORKSPACE_VAULT: VaultSpec = {
  id: "workspace",
  cipherDir: VAULT_CIPHER_DIR,
  mountPoint: WORKSPACE_MOUNT,
};

export const PENTEST_VAULT: VaultSpec = {
  id: "pentest",
  cipherDir: PENTEST_CIPHER_DIR,
  mountPoint: PENTEST_MOUNT,
};
