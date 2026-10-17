import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { WORKSPACE_MOUNT } from "@sdw/core/shared/layout";
import { isMountPoint, removeNote, writeNote } from "../src/notifications.ts";

let root: string;
beforeEach(() => {
  root = mkdtempSync(path.join(tmpdir(), "sdw-notes-"));
});
afterEach(() => rmSync(root, { recursive: true, force: true }));

describe("writeNote / removeNote", () => {
  it("creates the directory and writes the note", () => {
    const file = path.join(root, "a", "b", "note.md");
    expect(writeNote(file, "hello")).toBe(true);
    expect(readFileSync(file, "utf-8")).toBe("hello");
  });

  it("overwrites an earlier note", () => {
    const file = path.join(root, "note.md");
    writeNote(file, "one");
    writeNote(file, "two");
    expect(readFileSync(file, "utf-8")).toBe("two");
  });

  it("reports false instead of throwing when it cannot write", () => {
    const blocker = path.join(root, "file");
    writeFileSync(blocker, "x");
    expect(writeNote(path.join(blocker, "sub", "note.md"), "x")).toBe(false);
  });

  it("removing a note that is not there is fine", () => {
    const file = path.join(root, "note.md");
    expect(() => removeNote(file)).not.toThrow();
    writeNote(file, "x");
    removeNote(file);
    expect(existsSync(file)).toBe(false);
  });
});

describe("a note inside the workspace needs the vault mounted", () => {
  // A writable temp directory stands in for /workspace: the real root could not be
  // written to on most machines anyway, which would hide a missing guard.
  const guard = (mounted: boolean) => ({ root: path.join(root, "ws"), isMounted: () => mounted });
  const note = () => path.join(root, "ws", ".notifications", "x.md");

  it("is refused while nothing is mounted there, and creates nothing", () => {
    mkdirSync(path.join(root, "ws"));
    expect(writeNote(note(), "plaintext", guard(false))).toBe(false);
    expect(existsSync(path.join(root, "ws", ".notifications"))).toBe(false);
    expect(readdirSync(path.join(root, "ws"))).toEqual([]);
  });

  it("is written once the vault is mounted", () => {
    mkdirSync(path.join(root, "ws"));
    expect(writeNote(note(), "ok", guard(true))).toBe(true);
    expect(readFileSync(note(), "utf-8")).toBe("ok");
  });

  it("removing is a no-op while locked, so it cannot delete anything underneath the mount", () => {
    mkdirSync(path.join(root, "ws", ".notifications"), { recursive: true });
    writeFileSync(note(), "left behind");
    removeNote(note(), guard(false));
    expect(existsSync(note())).toBe(true);
    removeNote(note(), guard(true));
    expect(existsSync(note())).toBe(false);
  });

  it("a path outside the guarded root is not subject to it", () => {
    const other = path.join(root, "elsewhere", "x.md");
    expect(writeNote(other, "fine", guard(false))).toBe(true);
  });

  it("the default guard is the real workspace mount: refused here, nothing created", () => {
    // Holds on any machine without /workspace mounted (CI, a laptop, a locked container).
    if (isMountPoint(WORKSPACE_MOUNT)) return;
    expect(writeNote(`${WORKSPACE_MOUNT}/.notifications/x.md`, "plaintext")).toBe(false);
  });
});

describe("isMountPoint", () => {
  it("a plain directory is not one", () => {
    expect(isMountPoint(root)).toBe(false);
  });

  it("a path that does not exist is not one", () => {
    expect(isMountPoint(path.join(root, "missing"))).toBe(false);
  });

  it("the root of the filesystem is not 'a mount inside its parent'", () => {
    expect(isMountPoint("/")).toBe(false);
  });

  it.skipIf(!existsSync("/dev"))("/dev, a mount on every supported platform, is one", () => {
    expect(isMountPoint("/dev")).toBe(true);
  });
});
