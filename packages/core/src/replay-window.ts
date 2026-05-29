import { REPLAY_TOLERANCE_MS, REPLAY_WINDOW_SIZE } from "./constants.ts";

/**
 * Ring-buffer based replay window.
 * O(1) insert, O(1) lookup, fixed memory.
 */
export class ReplayWindow {
  private buffer: Array<{ nonceHex: string; ts: number } | null>;
  private head = 0;
  private nonceSet: Set<string>;
  private readonly maxSize: number;
  private readonly toleranceMs: number;

  constructor(maxSize = REPLAY_WINDOW_SIZE, toleranceMs = REPLAY_TOLERANCE_MS) {
    this.maxSize = maxSize;
    this.toleranceMs = toleranceMs;
    this.buffer = new Array(maxSize).fill(null);
    this.nonceSet = new Set();
  }

  /**
   * Check if an envelope should be accepted.
   * Returns true if OK, false if replay/out-of-window.
   */
  check(nonceHex: string, timestampMs: number, nowMs = Date.now()): boolean {
    if (Math.abs(nowMs - timestampMs) > this.toleranceMs) return false;
    if (this.nonceSet.has(nonceHex)) return false;

    // Evict oldest entry at head position
    const evicted = this.buffer[this.head];
    if (evicted) this.nonceSet.delete(evicted.nonceHex);

    // Insert new entry
    this.buffer[this.head] = { nonceHex, ts: timestampMs };
    this.nonceSet.add(nonceHex);
    this.head = (this.head + 1) % this.maxSize;

    return true;
  }

  reset(): void {
    this.buffer.fill(null);
    this.nonceSet.clear();
    this.head = 0;
  }
}
