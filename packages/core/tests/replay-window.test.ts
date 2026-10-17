import { describe, expect, it } from "bun:test";
import { ReplayWindow } from "../src/sync/replay-window.ts";

describe("ReplayWindow", () => {
  it("accepts fresh nonce", () => {
    const rw = new ReplayWindow(100, 300000);
    expect(rw.check("aabb", 1000, 1000)).toBe(true);
  });

  it("rejects duplicate nonce", () => {
    const rw = new ReplayWindow(100, 300000);
    rw.check("aabb", 1000, 1000);
    expect(rw.check("aabb", 1001, 1001)).toBe(false);
  });

  it("rejects out-of-window timestamp", () => {
    const rw = new ReplayWindow(100, 5000);
    expect(rw.check("cc", 1000, 10000)).toBe(false); // 9s skew > 5s tolerance
  });

  it("evicts oldest on overflow (ring buffer)", () => {
    const rw = new ReplayWindow(3, 300000);
    const now = 1000;
    rw.check("a", now, now);
    rw.check("b", now, now);
    rw.check("c", now, now);
    rw.check("d", now, now); // evicts "a"
    // "a" is no longer tracked, so it's accepted again
    expect(rw.check("a", now, now)).toBe(true);
  });

  it("reset clears all state", () => {
    const rw = new ReplayWindow(100, 300000);
    rw.check("x", 1000, 1000);
    rw.reset();
    expect(rw.check("x", 1000, 1000)).toBe(true);
  });
});

describe("ReplayWindow: malformed timestamps", () => {
  it("rejects NaN and infinities instead of letting them skip the tolerance check", () => {
    const rw = new ReplayWindow();
    for (const bad of [Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY]) {
      expect(rw.check("aa", bad, 1_000)).toBe(false);
    }
  });

  it("does not remember a rejected nonce", () => {
    const rw = new ReplayWindow();
    expect(rw.check("bb", Number.NaN, 1_000)).toBe(false);
    expect(rw.check("bb", 1_000, 1_000)).toBe(true);
  });
});
