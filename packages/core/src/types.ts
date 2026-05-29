// Shared type definitions

export type SyncState =
  | "vault-not-initialized"
  | "vault-locked"
  | "key-missing"
  | "idle"
  | "syncing"
  | "error"
  | "auth-expired";

export type EnvelopeOp = "put-chunk" | "put-finalize" | "delete" | "rename" | "list" | "handshake";

export interface Envelope {
  v: 1;
  op: EnvelopeOp;
  ts: number;
  nonce: Uint8Array;
  ciphertext: Uint8Array;
  tag: Uint8Array;
}

export interface FileState {
  path: string;
  mtime: number;
  size: number;
  hash: string;
}

export type SyncAction =
  | { type: "upload"; path: string }
  | { type: "download"; path: string }
  | { type: "delete-local"; path: string }
  | { type: "delete-remote"; path: string }
  | { type: "conflict"; path: string; localHash: string; remoteHash: string };

export interface WatchEvent {
  op: "add" | "change" | "unlink";
  path: string;
  mtime: number;
}

export interface ChunkStatus {
  fileId: string;
  receivedChunks: number[];
  totalChunks: number;
  createdAt: number;
}

export interface DoctorResult {
  component: string;
  status: "ok" | "warn" | "fail";
  detail?: string;
}
