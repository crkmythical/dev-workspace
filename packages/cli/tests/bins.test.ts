import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { installBins, launcher } from "../src/lib/bins.ts";

const CLI_ROOT = path.resolve(import.meta.dir, "..");
const pkg = JSON.parse(readFileSync(path.join(CLI_ROOT, "package.json"), "utf-8")) as {
  bin: Record<string, string>;
};

/** Entry files that run by themselves but are not commands a user types. */
const SERVICES = ["src/entrypoint-main.ts", "src/backup-watcher.ts"];

function entryFiles(): string[] {
  const top = readdirSync(path.join(CLI_ROOT, "src"))
    .filter((f) => f.endsWith(".ts"))
    .map((f) => `src/${f}`);
  const pluginDir = path.join(CLI_ROOT, "src", "plugins");
  const plugin = existsSync(pluginDir)
    ? readdirSync(pluginDir).flatMap((id) => {
        const commands = path.join(pluginDir, id, "commands");
        return existsSync(commands)
          ? readdirSync(commands)
              .filter((f) => f.endsWith(".ts"))
              .map((f) => `src/plugins/${id}/commands/${f}`)
          : [];
      })
    : [];
  return [...top, ...plugin].sort();
}

describe("the bin map is the list of commands", () => {
  it("every declared command points at a file that exists and is runnable", () => {
    for (const [name, rel] of Object.entries(pkg.bin)) {
      const file = path.join(CLI_ROOT, rel);
      expect(existsSync(file)).toBe(true);
      expect(readFileSync(file, "utf-8").startsWith("#!/usr/bin/env bun")).toBe(true);
      expect(name).toMatch(/^[a-z][a-z0-9-]*$/);
    }
  });

  it("no entry file is left out: each is a declared command or a known service", () => {
    const declared = new Set(Object.values(pkg.bin).map((rel) => rel.replace(/^\.\//, "")));
    const orphans = entryFiles().filter((f) => !declared.has(f) && !SERVICES.includes(f));
    // A new command that was not added to package.json would never reach PATH in the image.
    expect(orphans).toEqual([]);
  });

  it("the known services exist (the allowlist is not stale)", () => {
    for (const f of SERVICES) expect(existsSync(path.join(CLI_ROOT, f))).toBe(true);
  });

  it("two commands never share a name", () => {
    const names = Object.keys(pkg.bin);
    expect(new Set(names).size).toBe(names.length);
  });
});

describe("installBins", () => {
  let dest: string;
  beforeEach(() => {
    dest = mkdtempSync(path.join(tmpdir(), "sdw-bins-"));
  });
  afterEach(() => rmSync(dest, { recursive: true, force: true }));

  it("writes one executable launcher per command", () => {
    const names = installBins(CLI_ROOT, pkg.bin, dest);
    expect(names).toEqual(Object.keys(pkg.bin).sort());
    expect(readdirSync(dest).sort()).toEqual(names);
    for (const name of names) {
      const target = path.join(dest, name);
      expect(statSync(target).mode & 0o111).not.toBe(0);
    }
  });

  it("each launcher runs its command's entry file with bun and forwards arguments", () => {
    installBins(CLI_ROOT, pkg.bin, dest);
    const doctor = readFileSync(path.join(dest, "doctor"), "utf-8");
    expect(doctor).toBe(launcher(path.join(CLI_ROOT, "src/doctor.ts")));
    expect(doctor).toContain('"$@"');
    expect(doctor.startsWith("#!/bin/bash\nexec bun run ")).toBe(true);
  });

  it("creates the destination when it does not exist", () => {
    const nested = path.join(dest, "a", "b");
    installBins(CLI_ROOT, { hello: "./src/doctor.ts" }, nested);
    expect(existsSync(path.join(nested, "hello"))).toBe(true);
  });

  it("is idempotent", () => {
    installBins(CLI_ROOT, pkg.bin, dest);
    const first = readFileSync(path.join(dest, "unlock-vault"), "utf-8");
    installBins(CLI_ROOT, pkg.bin, dest);
    expect(readFileSync(path.join(dest, "unlock-vault"), "utf-8")).toBe(first);
  });
});
