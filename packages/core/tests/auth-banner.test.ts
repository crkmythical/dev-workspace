import { describe, expect, it } from "bun:test";
import fc from "fast-check";
import type { EgressReport } from "../src/egress/index.ts";
import {
  type BannerInput,
  parseCodeServerPassword,
  renderBanner,
  renderCodeServerConfig,
  resolveMasterPassword,
} from "../src/workspace/index.ts";

const generate = () => "generated-pw";

describe("resolveMasterPassword", () => {
  it("an explicit PASSWORD wins, even over an existing config, and is written out", () => {
    const r = resolveMasterPassword({
      envPassword: "from-env",
      existingConfig: renderCodeServerConfig("old", 8082),
      generate,
    });
    expect(r).toEqual({ value: "from-env", source: "env", writeConfig: true });
  });

  it("keeps the password already in code-server's config, so restarts do not change it", () => {
    const r = resolveMasterPassword({
      envPassword: undefined,
      existingConfig: renderCodeServerConfig("kept", 8082),
      generate,
    });
    expect(r).toEqual({ value: "kept", source: "existing", writeConfig: false });
  });

  it("generates and writes one when there is no config yet", () => {
    const r = resolveMasterPassword({
      envPassword: undefined,
      existingConfig: undefined,
      generate,
    });
    expect(r).toEqual({ value: "generated-pw", source: "generated", writeConfig: true });
  });

  it("a config without a usable password is replaced, so what is shown is what is enforced", () => {
    for (const existingConfig of ["auth: password\n", "password:\n", "password:   \n", ""]) {
      const r = resolveMasterPassword({ envPassword: undefined, existingConfig, generate });
      expect(r).toEqual({ value: "generated-pw", source: "generated", writeConfig: true });
    }
  });

  it("an empty PASSWORD counts as unset", () => {
    const r = resolveMasterPassword({ envPassword: "", existingConfig: undefined, generate });
    expect(r.source).toBe("generated");
  });

  it("refuses a PASSWORD with a line break", () => {
    for (const bad of ["a\nb", "a\r\nb", "line\r"]) {
      expect(() =>
        resolveMasterPassword({ envPassword: bad, existingConfig: undefined, generate }),
      ).toThrow("single line");
    }
  });
});

describe("code-server config", () => {
  it("renders the fields code-server needs", () => {
    expect(renderCodeServerConfig("pw", 8082)).toBe(
      'bind-addr: 127.0.0.1:8082\nauth: password\npassword: "pw"\ncert: false\n',
    );
  });

  it("reads back any single-line password it wrote (YAML-special characters included)", () => {
    const singleLine = fc.string().filter((s) => !/[\r\n]/.test(s) && s.trim() !== "");
    fc.assert(
      fc.property(singleLine, (pw) => {
        expect(parseCodeServerPassword(renderCodeServerConfig(pw, 8082))).toBe(pw);
      }),
    );
  });

  it("survives the characters that would change an unquoted YAML value", () => {
    for (const pw of [
      "a #b",
      "x: y",
      "- z",
      "'q'",
      '"d"',
      "a\\b",
      "100%",
      "{braces}",
      "tab\there",
    ]) {
      expect(parseCodeServerPassword(renderCodeServerConfig(pw, 8082))).toBe(pw);
    }
  });

  it("still reads the unquoted form older versions wrote", () => {
    expect(parseCodeServerPassword("bind-addr: x\npassword: abc123\ncert: false\n")).toBe("abc123");
    expect(parseCodeServerPassword("password:   spaced   \n")).toBe("spaced");
    expect(parseCodeServerPassword("password: 'it''s'\n")).toBe("it's");
    expect(parseCodeServerPassword("password: abc\r\ncert: false\r\n")).toBe("abc");
  });

  it("returns null when there is no password", () => {
    expect(parseCodeServerPassword("auth: none\n")).toBeNull();
    expect(parseCodeServerPassword('password: ""\n')).toBeNull();
    expect(parseCodeServerPassword('password: "unterminated\n')).toBeNull();
  });
});

describe("renderBanner", () => {
  const confirmed: EgressReport = { status: "confirmed", checkedAt: "t", providerNodes: 9 };
  const base: BannerInput = {
    password: "s3cret",
    stacks: ["selkies", "vnc"],
    vaultMounted: true,
    egress: confirmed,
    degraded: [],
  };
  const text = (input: Partial<BannerInput> = {}) => renderBanner({ ...base, ...input }).join("\n");

  it("draws a box whose every line is the same width", () => {
    const lines = renderBanner(base).filter((l) => /^[╔║╠╚]/.test(l));
    expect(lines.length).toBeGreaterThan(5);
    expect(new Set(lines.map((l) => [...l].length)).size).toBe(1);
  });

  it("shows how to log in and where", () => {
    const out = text();
    expect(out).toContain("Password:  s3cret");
    expect(out).toContain("user / s3cret");
    expect(out).toContain("http://localhost:18080/desktop/");
    expect(out).toContain("http://localhost:18080/vnc/");
  });

  it("lists only the desktops the image contains", () => {
    expect(text({ stacks: ["vnc"] })).not.toContain("/desktop/");
    expect(text({ stacks: ["selkies"] })).not.toContain("/vnc/");
  });

  it("a healthy start has no attention lines", () => {
    const out = text();
    expect(out).toContain("Vault:     unlocked");
    expect(out).toContain("Egress:    confirmed");
    expect(out).not.toContain("⚠");
  });

  it("a locked vault says so, and what to do", () => {
    const out = text({ vaultMounted: false });
    expect(out).toContain("Vault:     LOCKED");
    expect(out).toContain("⚠ Vault is locked: run `unlock-vault`");
  });

  it("degraded and down egress are in the box AND explained below it", () => {
    const degraded = text({ egress: { ...confirmed, status: "degraded" } });
    expect(degraded).toContain("Egress:    DEGRADED");
    expect(degraded).toContain("⚠ Egress DEGRADED:");
    const down = text({ egress: { ...confirmed, status: "down" } });
    expect(down).toContain("Egress:    DOWN");
    expect(down).toContain("⚠ Egress DOWN:");
  });

  it("an egress step that failed outright reads as unknown, not as fine", () => {
    const out = text({ egress: undefined });
    expect(out).toContain("Egress:    UNKNOWN");
    expect(out).toContain("⚠ Egress was not checked");
  });

  it("names the start-up steps that failed", () => {
    const out = text({ degraded: ["prepare-desktops", "prepare-backup"] });
    expect(out).toContain("prepare-desktops, prepare-backup");
  });
});
