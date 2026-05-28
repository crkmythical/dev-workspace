import type { FileState, SyncAction } from "./types.ts";

/**
 * Pure three-way reconciliation algorithm.
 * Deterministic, idempotent, no IO.
 */
export function reconcile(
  local: FileState[],
  remote: FileState[],
  baseline: FileState[],
): SyncAction[] {
  const localMap = new Map(local.map((f) => [f.path, f]));
  const remoteMap = new Map(remote.map((f) => [f.path, f]));
  const baselineMap = new Map(baseline.map((f) => [f.path, f]));

  const allPaths = new Set([
    ...localMap.keys(),
    ...remoteMap.keys(),
    ...baselineMap.keys(),
  ]);

  const actions: SyncAction[] = [];

  for (const path of allPaths) {
    const l = localMap.get(path);
    const r = remoteMap.get(path);
    const b = baselineMap.get(path);

    if (l && !r && !b) {
      actions.push({ type: "upload", path });
    } else if (!l && r && !b) {
      actions.push({ type: "download", path });
    } else if (b && !l && r) {
      actions.push({ type: "delete-remote", path });
    } else if (b && l && !r) {
      actions.push({ type: "delete-local", path });
    } else if (l && r && l.hash !== r.hash) {
      const localChanged = !b || l.hash !== b.hash;
      const remoteChanged = !b || r.hash !== b.hash;

      if (localChanged && remoteChanged) {
        actions.push({ type: "conflict", path, localHash: l.hash, remoteHash: r.hash });
      } else if (localChanged) {
        actions.push({ type: "upload", path });
      } else if (remoteChanged) {
        actions.push({ type: "download", path });
      }
    }
  }

  return actions.sort((a, b) => a.path.localeCompare(b.path));
}
