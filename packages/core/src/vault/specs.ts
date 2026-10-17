/**
 * Deployment default for the workspace vault. Only composition roots (cli / server /
 * entrypoint) should import it; plugins that own further vaults define their own
 * (see ../plugins).
 */
import { WORKSPACE_MOUNT } from "../shared/layout.ts";
import type { VaultSpec } from "./domain.ts";
import { VAULT_CIPHER_DIR } from "./layout.ts";

export const WORKSPACE_VAULT: VaultSpec = {
  id: "workspace",
  cipherDir: VAULT_CIPHER_DIR,
  mountPoint: WORKSPACE_MOUNT,
};
