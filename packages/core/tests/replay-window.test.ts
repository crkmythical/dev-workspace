import { describe, expect, it } from "bun:test";
import { ReplayWindow } from "../src/replay-window.ts";

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
