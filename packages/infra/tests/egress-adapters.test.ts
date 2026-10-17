import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import type { EgressReport } from "@sdw/core/egress";
import { readEgressStatus, syncEgressNotice, writeEgressStatus } from "../src/egress.ts";

let root: string;
beforeEach(() => {
  root = mkdtempSync(path.join(tmpdir(), "sdw-egress-"));
});
afterEach(() => rmSync(root, { recursive: true, force: true }));

const report = (status: EgressReport["status"]): EgressReport => ({
  status,
  checkedAt: "2026-10-17T12:00:00.000Z",
  providerNodes: 3,
});

describe("the status file", () => {
  it("what is written is read back", () => {
    const file = path.join(root, "egress-status");
    writeEgressStatus(report("degraded"), file);
    expect(readEgressStatus(file)).toEqual(report("degraded"));
  });

  it("a missing or damaged file reads as no report", () => {
    const file = path.join(root, "egress-status");
    expect(readEgressStatus(file)).toBeNull();
    writeFileSync(file, "{not json");
    expect(readEgressStatus(file)).toBeNull();
  });
});

describe("the egress note", () => {
  const note = () => path.join(root, "notes", "egress-degraded.md");

  it("is written while egress is not confirmed", () => {
    expect(syncEgressNotice(report("down"), note())).toBe("written");
    expect(readFileSync(note(), "utf-8")).toContain("Egress DOWN");
    expect(syncEgressNotice(report("degraded"), note())).toBe("written");
    expect(readFileSync(note(), "utf-8")).toContain("Egress DEGRADED");
  });

  it("is removed once egress is confirmed", () => {
    syncEgressNotice(report("down"), note());
    expect(existsSync(note())).toBe(true);
    expect(syncEgressNotice(report("confirmed"), note())).toBe("removed");
    expect(existsSync(note())).toBe(false);
  });

  it("confirmed with no earlier note is just fine", () => {
    expect(syncEgressNotice(report("confirmed"), note())).toBe("removed");
  });

  it("reports when it cannot write", () => {
    const blocker = path.join(root, "blocker");
    writeFileSync(blocker, "x");
    expect(syncEgressNotice(report("down"), path.join(blocker, "sub", "n.md"))).toBe("unavailable");
  });
});
