import { describe, expect, it } from "bun:test";
import { ALL_VAULTS, PLUGINS } from "../src/plugins/index.ts";
import { WORKSPACE_VAULT } from "../src/vault/specs.ts";

describe("plugin registry", () => {
  it("every plugin has its own id", () => {
    const ids = PLUGINS.map((p) => p.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("ALL_VAULTS is the workspace first, then every plugin's volumes", () => {
    expect(ALL_VAULTS[0]).toBe(WORKSPACE_VAULT);
    expect(ALL_VAULTS.slice(1)).toEqual(PLUGINS.flatMap((p) => [...p.vaults]));
  });

  it("no two volumes share an id, a ciphertext directory or a mount point", () => {
    // destroy shreds every volume's key and unmounts every mount point: a collision would
    // shred the wrong key or unmount the wrong thing.
    for (const field of ["id", "cipherDir", "mountPoint"] as const) {
      const values = ALL_VAULTS.map((v) => v[field]);
      expect(new Set(values).size).toBe(values.length);
    }
  });

  it("no volume lives inside another one's directories", () => {
    const inside = (a: string, b: string) => a === b || a.startsWith(`${b}/`);
    for (const a of ALL_VAULTS) {
      for (const b of ALL_VAULTS) {
        if (a === b) continue;
        expect(inside(a.mountPoint, b.mountPoint)).toBe(false);
        expect(inside(a.cipherDir, b.cipherDir)).toBe(false);
        expect(inside(a.cipherDir, b.mountPoint)).toBe(false);
        expect(inside(a.mountPoint, b.cipherDir)).toBe(false);
      }
    }
  });

  it("a plugin cannot claim the workspace's id", () => {
    for (const plugin of PLUGINS) {
      for (const vault of plugin.vaults) expect(vault.id).not.toBe(WORKSPACE_VAULT.id);
    }
  });
});
