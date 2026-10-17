import { describe, expect, test } from "bun:test";
import { RateLimiter } from "../src/rate-limit.ts";

const CONFIG = { windowMs: 60_000, perKeyMax: 3, globalMax: 10 };

describe("RateLimiter — per-key budget", () => {
  test("allows up to perKeyMax attempts, rejects the next", () => {
    const rl = new RateLimiter(CONFIG);
    const t = 1_000_000;
    expect(rl.check("ip-a", t)).toBe(false); // 1
    expect(rl.check("ip-a", t)).toBe(false); // 2
    expect(rl.check("ip-a", t)).toBe(false); // 3
    expect(rl.check("ip-a", t)).toBe(true); // 4 → over perKeyMax
  });

  test("independent keys have independent per-key budgets", () => {
    const rl = new RateLimiter(CONFIG);
    const t = 2_000_000;
    for (let i = 0; i < 3; i++) expect(rl.check("ip-a", t)).toBe(false);
    // ip-b is fresh — not affected by ip-a's exhausted budget
    expect(rl.check("ip-b", t)).toBe(false);
  });

  test("per-key window resets after windowMs elapses", () => {
    const rl = new RateLimiter(CONFIG);
    const t0 = 3_000_000;
    for (let i = 0; i < 3; i++) expect(rl.check("ip-a", t0)).toBe(false);
    expect(rl.check("ip-a", t0)).toBe(true); // over budget within the window
    // After the window, the key's budget is fresh again
    const t1 = t0 + CONFIG.windowMs + 1;
    expect(rl.check("ip-a", t1)).toBe(false);
  });
});

describe("RateLimiter — global backstop (header-spoofing resistance)", () => {
  test("a flood of unique keys still hits the global cap", () => {
    const rl = new RateLimiter(CONFIG);
    const t = 4_000_000;
    // Simulate X-Forwarded-For rotation: every attempt uses a fresh key, so the
    // per-key budget is never reached — only the global cap bounds the attacker.
    let rejected = 0;
    for (let i = 0; i < 50; i++) {
      if (rl.check(`spoofed-${i}`, t)) rejected++;
    }
    // globalMax attempts pass; the remainder are rejected by the global window.
    expect(rejected).toBe(50 - CONFIG.globalMax);
  });

  test("global window also resets after windowMs", () => {
    const rl = new RateLimiter(CONFIG);
    const t0 = 5_000_000;
    for (let i = 0; i < CONFIG.globalMax; i++) expect(rl.check(`k-${i}`, t0)).toBe(false);
    expect(rl.check("k-overflow", t0)).toBe(true); // global exhausted
    const t1 = t0 + CONFIG.windowMs + 1;
    expect(rl.check("k-fresh", t1)).toBe(false); // global reset
  });
});

describe("RateLimiter — reset()", () => {
  test("reset clears per-key and global state", () => {
    const rl = new RateLimiter(CONFIG);
    const t = 6_000_000;
    for (let i = 0; i < 3; i++) rl.check("ip-a", t);
    expect(rl.check("ip-a", t)).toBe(true);
    rl.reset();
    expect(rl.check("ip-a", t)).toBe(false);
  });
});
