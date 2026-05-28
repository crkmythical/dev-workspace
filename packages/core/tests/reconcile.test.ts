import { describe, it, expect } from "bun:test";
import { reconcile } from "../src/reconcile.ts";
import type { FileState } from "../src/types.ts";

describe("reconcile", () => {
  it("returns empty for identical states", () => {
    const state: FileState[] = [{ path: "a.txt", mtime: 1, size: 10, hash: "abc" }];
    expect(reconcile(state, state, state)).toEqual([]);
  });

  it("detects new local file", () => {
    const local: FileState[] = [{ path: "new.txt", mtime: 1, size: 5, hash: "x" }];
    const actions = reconcile(local, [], []);
    expect(actions).toEqual([{ type: "upload", path: "new.txt" }]);
  });

  it("detects new remote file", () => {
    const remote: FileState[] = [{ path: "remote.txt", mtime: 1, size: 5, hash: "y" }];
    const actions = reconcile([], remote, []);
    expect(actions).toEqual([{ type: "download", path: "remote.txt" }]);
  });

  it("detects conflict when both sides changed", () => {
    const baseline: FileState[] = [{ path: "f.txt", mtime: 1, size: 5, hash: "base" }];
    const local: FileState[] = [{ path: "f.txt", mtime: 2, size: 5, hash: "local" }];
    const remote: FileState[] = [{ path: "f.txt", mtime: 3, size: 5, hash: "remote" }];
    const actions = reconcile(local, remote, baseline);
    expect(actions).toEqual([{ type: "conflict", path: "f.txt", localHash: "local", remoteHash: "remote" }]);
  });

  it("actions are sorted by path", () => {
    const local: FileState[] = [
      { path: "z.txt", mtime: 1, size: 1, hash: "z" },
      { path: "a.txt", mtime: 1, size: 1, hash: "a" },
    ];
    const actions = reconcile(local, [], []);
    expect(actions[0].path).toBe("a.txt");
    expect(actions[1].path).toBe("z.txt");
  });
});
