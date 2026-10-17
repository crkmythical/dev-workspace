/**
 * Fixed-window rate limiter with a per-key budget AND a global backstop.
 *
 * Rationale: endpoints like self-destruct derive the client key from a
 * client-influenced header (CF-Connecting-IP / X-Forwarded-For). A per-key
 * budget alone can be defeated by rotating that header, so we add a GLOBAL
 * window that caps total attempts across ALL keys within the window. The
 * global cap cannot be bypassed by header spoofing.
 *
 * Trade-off: a flood of spoofed-key requests can exhaust the global budget and
 * temporarily reject every client for the rest of the window. For self-destruct
 * this fails in the safe direction (blocking the irreversible action), and the
 * local CLI path (`self-destruct`) is unaffected.
 *
 * Pure and deterministic given an injected `nowMs`, so the window and
 * bypass-resistance invariants are unit-testable without real timers.
 */
export interface RateLimitConfig {
  /** Window length in milliseconds. */
  windowMs: number;
  /** Max attempts per key within a window. */
  perKeyMax: number;
  /** Max attempts across ALL keys within a window (spoof-resistant backstop). */
  globalMax: number;
}

interface WindowState {
  count: number;
  resetAt: number;
}

export class RateLimiter {
  private readonly perKey = new Map<string, WindowState>();
  private global: WindowState | null = null;
  private readonly config: RateLimitConfig;

  constructor(config: RateLimitConfig) {
    this.config = config;
  }

  /**
   * Record an attempt for `key` at `nowMs`. Returns true if the attempt should
   * be REJECTED (over the per-key or global budget).
   */
  check(key: string, nowMs: number = Date.now()): boolean {
    // Global backstop first — counts every attempt regardless of key.
    if (!this.global || nowMs > this.global.resetAt) {
      this.global = { count: 1, resetAt: nowMs + this.config.windowMs };
    } else {
      this.global.count++;
      if (this.global.count > this.config.globalMax) return true;
    }

    // Per-key window.
    const entry = this.perKey.get(key);
    if (!entry || nowMs > entry.resetAt) {
      this.perKey.set(key, { count: 1, resetAt: nowMs + this.config.windowMs });
      return false;
    }
    entry.count++;
    return entry.count > this.config.perKeyMax;
  }

  /** Drop all state (test helper / long-running hygiene). */
  reset(): void {
    this.perKey.clear();
    this.global = null;
  }
}
