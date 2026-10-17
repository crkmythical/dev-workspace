import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { createFileFailureTracker } from "../src/failure-tracker.ts";

let root: string;
beforeEach(() => {
  root = mkdtempSync(path.join(tmpdir(), "sdw-failure-"));
});
afterEach(() => rmSync(root, { recursive: true, force: true }));

function tracker(threshold = 3) {
  return createFileFailureTracker({
    stateDir: path.join(root, "state"),
    threshold,
    notificationFile: "job-failure.md",
    notificationBody: (n) => `failed ${n} times\n`,
    notificationsDir: path.join(root, "notifications"),
  });
}
const note = () => path.join(root, "notifications", "job-failure.md");

describe("FailureTracker (file backed)", () => {
  it("starts at zero", () => {
    expect(tracker().consecutiveFailures()).toBe(0);
  });

  it("counts consecutive failures and persists them across instances", () => {
    tracker().recordFailure();
    expect(tracker().recordFailure().count).toBe(2);
    expect(tracker().consecutiveFailures()).toBe(2);
  });

  it("stays quiet below the threshold, then notifies at it", () => {
    const t = tracker(3);
    expect(t.recordFailure()).toEqual({ count: 1, notified: false });
    expect(t.recordFailure()).toEqual({ count: 2, notified: false });
    expect(existsSync(note())).toBe(false);
    expect(t.recordFailure()).toEqual({ count: 3, notified: true });
    expect(readFileSync(note(), "utf-8")).toBe("failed 3 times\n");
  });

  it("keeps the notification current as failures continue", () => {
    const t = tracker(1);
    t.recordFailure();
    t.recordFailure();
    expect(readFileSync(note(), "utf-8")).toBe("failed 2 times\n");
  });

  it("a success resets the counter and records when it happened", () => {
    const t = tracker();
    t.recordFailure();
    t.recordSuccess();
    expect(t.consecutiveFailures()).toBe(0);
    expect(readFileSync(path.join(root, "state", "last-success"), "utf-8")).toMatch(
      /^\d{4}-\d\d-\d\dT/,
    );
  });

  it("a success clears the notification the failures left, and a success without one is fine", () => {
    const t = tracker(1);
    t.recordSuccess(); // no note yet: nothing to clear, nothing thrown
    t.recordFailure();
    expect(existsSync(note())).toBe(true);
    t.recordSuccess();
    expect(existsSync(note())).toBe(false);
  });

  it("a corrupt counter file reads as zero rather than throwing", () => {
    tracker(); // creates the state dir
    writeFileSync(path.join(root, "state", "consecutive-failures"), "not a number");
    expect(tracker().consecutiveFailures()).toBe(0);
  });

  it("an unwritable notifications location is not fatal", () => {
    const blocked = path.join(root, "blocked");
    writeFileSync(blocked, "a file where a directory is needed");
    const t = createFileFailureTracker({
      stateDir: path.join(root, "state"),
      threshold: 1,
      notificationFile: "x.md",
      notificationBody: () => "x",
      notificationsDir: path.join(blocked, "sub"),
    });
    expect(t.recordFailure()).toEqual({ count: 1, notified: false });
  });
});
