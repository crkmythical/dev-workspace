import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { enableAutostart, withAutostart } from "../src/supervisor.ts";

let root: string;
beforeEach(() => {
  root = mkdtempSync(path.join(tmpdir(), "sdw-supervisor-"));
});
afterEach(() => rmSync(root, { recursive: true, force: true }));

const CONF = `# xfce(200). All autostart=false (on-demand).
[program:desktop-xvfb]
command=/usr/bin/Xvfb :1
autostart=false
autorestart=true

[program:desktop-xfce]
command=/usr/local/bin/start-xfce.sh
autostart=false
`;

describe("withAutostart", () => {
  it("flips every autostart=false directive", () => {
    const out = withAutostart(CONF);
    expect(out.match(/^autostart=true$/gm)).toHaveLength(2);
    expect(out).not.toMatch(/^autostart=false$/m);
  });

  it("leaves comments and other settings alone", () => {
    const out = withAutostart(CONF);
    expect(out).toContain("# xfce(200). All autostart=false (on-demand).");
    expect(out).toContain("autorestart=true");
    expect(out).toContain("command=/usr/bin/Xvfb :1");
  });

  it("tolerates spaces around the equals sign", () => {
    expect(withAutostart("autostart = false\n")).toBe("autostart = true\n");
  });

  it("is idempotent", () => {
    expect(withAutostart(withAutostart(CONF))).toBe(withAutostart(CONF));
  });

  it("does not touch a different directive that merely contains the word", () => {
    expect(withAutostart("autostartsecs=false\n")).toBe("autostartsecs=false\n");
  });
});

describe("enableAutostart", () => {
  it("rewrites the file and says it changed", () => {
    const file = path.join(root, "vnc.conf");
    writeFileSync(file, CONF);
    expect(enableAutostart(file)).toBe(true);
    expect(readFileSync(file, "utf-8")).toBe(withAutostart(CONF));
  });

  it("reports no change the second time", () => {
    const file = path.join(root, "vnc.conf");
    writeFileSync(file, CONF);
    enableAutostart(file);
    expect(enableAutostart(file)).toBe(false);
  });

  it("skips a file the image does not have", () => {
    expect(enableAutostart(path.join(root, "absent.conf"))).toBe(false);
  });
});
