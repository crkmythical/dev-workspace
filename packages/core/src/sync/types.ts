/** Types of the three-way file reconciliation. */

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
