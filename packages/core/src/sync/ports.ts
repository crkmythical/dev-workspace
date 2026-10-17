/**
 * Sync ports: what the receive use cases need from the outside world.
 * Implemented by adapters in @sdw/infra; replaced by fakes in tests.
 */

/** Raw AES-256 key bytes. */
export type SyncKey = Uint8Array;

export interface KeyProvider {
  /** The current sync key, or null while the vault is locked (no key is available). */
  current(): Promise<SyncKey | null>;
}

export interface AeadPort {
  /** Authenticated decryption. Null when authentication fails (wrong key, AAD or tampering). */
  open(
    key: SyncKey,
    nonce: Uint8Array,
    ciphertext: Uint8Array,
    tag: Uint8Array,
    aad: Uint8Array,
  ): Promise<Uint8Array | null>;
}

export interface SharedFile {
  name: string;
  size: number;
  mtime: number;
}

/** Staging area for chunked uploads plus the shared directory they end up in. */
export interface UploadStore {
  appendChunk(fileId: string, data: Uint8Array): Promise<void>;
  hasChunks(fileId: string): Promise<boolean>;
  sha256Hex(fileId: string): Promise<string>;
  discard(fileId: string): Promise<void>;
  /** Move the staged file to `relativePath` under the shared directory. */
  commit(fileId: string, relativePath: string): Promise<void>;
  list(): Promise<SharedFile[]>;
}
