import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import { existsSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { type VaultSpec, WORKSPACE_VAULT } from "@sdw/core/vault";
import { deriveDestructKey, destructKeyToPassphrase } from "../src/destruct-key.ts";
import { remoteDestructKeyHook, syncCredentialHook } from "../src/unlock-hooks.ts";

/** Some other vault (a plugin's): these hooks only ever apply to the workspace. */
const OTHER_VAULT: VaultSpec = { id: "other", cipherDir: "/c/other", mountPoint: "/m/other" };

/**
 * The two feature-gated unlock hooks, run against real files in a temp dir.
 * "Enabled" provisions credential material; "disabled" removes stale material,
 * so a flag that is switched off is also a clean-up.
 */

let dir: string;
beforeEach(() => {
  dir = mkdtempSync(path.join(tmpdir(), "sdw-hooks-"));
});
afterEach(() => {
  rmSync(dir, { recursive: true, force: true });
});

const mode = (file: string) => statSync(file).mode & 0o777;

describe("syncCredentialHook", () => {
  it("enabled: writes the vault passphrase, readable only by its owner", async () => {
    const target = path.join(dir, ".credentials", "sync-passphrase");
    const hook = syncCredentialHook({ enabled: true, path: target });
    await hook.run({ spec: WORKSPACE_VAULT, passphrase: "first-passphrase" });
    expect(readFileSync(target, "utf-8")).toBe("first-passphrase");
    expect(mode(target)).toBe(0o600);
  });

  it("enabled: a rotated passphrase overwrites the old one and keeps the mode", async () => {
    const target = path.join(dir, "sync-passphrase");
    const hook = syncCredentialHook({ enabled: true, path: target });
    await hook.run({ spec: WORKSPACE_VAULT, passphrase: "old-passphrase" });
    await hook.run({ spec: WORKSPACE_VAULT, passphrase: "new-passphrase" });
    expect(readFileSync(target, "utf-8")).toBe("new-passphrase");
    expect(mode(target)).toBe(0o600);
  });

  it("disabled: removes a stale credential and says so once", async () => {
    const target = path.join(dir, "sync-passphrase");
    writeFileSync(target, "left-over");
    const messages: string[] = [];
    const hook = syncCredentialHook({
      enabled: false,
      path: target,
      announce: (m) => messages.push(m),
    });
    await hook.run({ spec: WORKSPACE_VAULT, passphrase: "irrelevant" });
    expect(existsSync(target)).toBe(false);
    expect(messages).toHaveLength(1);
    await hook.run({ spec: WORKSPACE_VAULT, passphrase: "irrelevant" });
    expect(messages).toHaveLength(1);
  });

  it("disabled: never writes anything", async () => {
    const target = path.join(dir, "sync-passphrase");
    await syncCredentialHook({ enabled: false, path: target }).run({
      spec: WORKSPACE_VAULT,
      passphrase: "irrelevant",
    });
    expect(existsSync(target)).toBe(false);
  });

  it("only applies to the workspace vault", async () => {
    const target = path.join(dir, "sync-passphrase");
    await syncCredentialHook({ enabled: true, path: target }).run({
      spec: OTHER_VAULT,
      passphrase: "irrelevant",
    });
    expect(existsSync(target)).toBe(false);
  });
});

describe("remoteDestructKeyHook", () => {
  const cost = 4; // bcrypt minimum: keeps the test fast, same code path

  function makeHook(enabled: boolean, messages: string[]) {
    return remoteDestructKeyHook({
      enabled,
      cost,
      hashPath: path.join(dir, "self-destruct", "key-hash"),
      stateDir: path.join(dir, "self-destruct"),
      announce: (m) => messages.push(m),
    });
  }
  const hashFile = () => path.join(dir, "self-destruct", "key-hash");
  const derived = (vaultPassphrase: string) =>
    destructKeyToPassphrase(deriveDestructKey(vaultPassphrase));

  it("enabled: stores a bcrypt hash that verifies the derived passphrase", async () => {
    const messages: string[] = [];
    await makeHook(true, messages).run({ spec: WORKSPACE_VAULT, passphrase: "vault-passphrase" });
    const hash = readFileSync(hashFile(), "utf-8");
    expect(await Bun.password.verify(derived("vault-passphrase"), hash)).toBe(true);
    expect(await Bun.password.verify("not-the-passphrase", hash)).toBe(false);
  });

  it("enabled: shows the passphrase the first time only", async () => {
    const messages: string[] = [];
    const hook = makeHook(true, messages);
    await hook.run({ spec: WORKSPACE_VAULT, passphrase: "vault-passphrase" });
    expect(messages.join("")).toContain(derived("vault-passphrase"));
    expect(messages).toHaveLength(1);
    await hook.run({ spec: WORKSPACE_VAULT, passphrase: "vault-passphrase" });
    expect(messages).toHaveLength(1);
  });

  it("enabled: a rotated vault passphrase rotates the destruct passphrase", async () => {
    const hook = makeHook(true, []);
    await hook.run({ spec: WORKSPACE_VAULT, passphrase: "old-vault-passphrase" });
    await hook.run({ spec: WORKSPACE_VAULT, passphrase: "new-vault-passphrase" });
    const hash = readFileSync(hashFile(), "utf-8");
    expect(await Bun.password.verify(derived("new-vault-passphrase"), hash)).toBe(true);
    expect(await Bun.password.verify(derived("old-vault-passphrase"), hash)).toBe(false);
  });

  it("disabled: removes a stale hash so the endpoint stays dormant", async () => {
    const messages: string[] = [];
    await makeHook(true, []).run({ spec: WORKSPACE_VAULT, passphrase: "vault-passphrase" });
    expect(existsSync(hashFile())).toBe(true);
    await makeHook(false, messages).run({ spec: WORKSPACE_VAULT, passphrase: "vault-passphrase" });
    expect(existsSync(hashFile())).toBe(false);
    expect(messages).toHaveLength(1);
  });

  it("disabled: is silent when there is nothing to remove", async () => {
    const messages: string[] = [];
    await makeHook(false, messages).run({ spec: WORKSPACE_VAULT, passphrase: "vault-passphrase" });
    expect(messages).toEqual([]);
    expect(existsSync(hashFile())).toBe(false);
  });

  it("only applies to the workspace vault", async () => {
    const messages: string[] = [];
    await makeHook(true, messages).run({ spec: OTHER_VAULT, passphrase: "vault-passphrase" });
    expect(existsSync(hashFile())).toBe(false);
    expect(messages).toEqual([]);
  });
});
