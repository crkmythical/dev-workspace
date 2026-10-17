/** Where the two vaults keep their ciphertext, and vault-related runtime state. */
import { RUN_DIR } from "../shared/layout.ts";

export const VAULT_CIPHER_DIR = "/vault/cipher";

export const PENTEST_CIPHER_DIR = "/pentest/cipher";
export const PENTEST_MOUNT = "/pentest/rootfs";
/** Tor SOCKS port inside the pentest rootfs (used by its proxychains/torrc templates). */
export const PENTEST_TOR_PORT = 9050;

export const STATE_SOCKET_PATH = `${RUN_DIR}/vault-state.sock`;
export const VAULT_SYNC_STATE_DIR = `${RUN_DIR}/vault-sync`;
