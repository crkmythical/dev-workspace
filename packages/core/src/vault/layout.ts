/** Where the workspace vault keeps its ciphertext, and vault-related runtime state. */
import { RUN_DIR } from "../shared/layout.ts";

export const VAULT_CIPHER_DIR = "/vault/cipher";

export const STATE_SOCKET_PATH = `${RUN_DIR}/vault-state.sock`;
export const VAULT_SYNC_STATE_DIR = `${RUN_DIR}/vault-sync`;
