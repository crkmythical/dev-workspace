import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import { mkdirSync, mkdtempSync, realpathSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { stepOutOf } from "../src/gocryptfs.ts";

/**
 * A process releasing a mount must not be standing in it. Found by running self-destruct
 * from a terminal inside the container: the unmount went through lazily, `getcwd` then
 * failed, and every later step (erasing the keys among them) died with ENOENT.
 */
let original: string;
let root: string;

beforeEach(() => {
  original = process.cwd();
  root = realpathSync(mkdtempSync(path.join(tmpdir(), "sdw-stepout-")));
});

afterEach(() => {
  process.chdir(original);
  rmSync(root, { recursive: true, force: true });
});

describe("stepOutOf", () => {
  it("leaves the mount point itself", () => {
    const mount = path.join(root, "workspace");
    mkdirSync(mount);
    process.chdir(mount);
    stepOutOf(mount);
    expect(process.cwd()).toBe("/");
  });

  it("leaves a directory below the mount point", () => {
    const mount = path.join(root, "workspace");
    const deep = path.join(mount, "project", "src");
    mkdirSync(deep, { recursive: true });
    process.chdir(deep);
    stepOutOf(mount);
    expect(process.cwd()).toBe("/");
  });

  it("stays where it is when the mount is somewhere else", () => {
    const mount = path.join(root, "workspace");
    const elsewhere = path.join(root, "elsewhere");
    mkdirSync(mount);
    mkdirSync(elsewhere);
    process.chdir(elsewhere);
    stepOutOf(mount);
    expect(process.cwd()).toBe(elsewhere);
  });

  it("does not take a sibling that shares the name prefix for the mount", () => {
    const mount = path.join(root, "workspace");
    const sibling = path.join(root, "workspace-other");
    mkdirSync(mount);
    mkdirSync(sibling);
    process.chdir(sibling);
    stepOutOf(mount);
    expect(process.cwd()).toBe(sibling);
  });

  it("copes with a working directory that has already vanished", () => {
    const mount = path.join(root, "workspace");
    mkdirSync(mount);
    process.chdir(mount);
    rmSync(mount, { recursive: true });
    stepOutOf(mount);
    expect(process.cwd()).toBe("/");
  });
});
