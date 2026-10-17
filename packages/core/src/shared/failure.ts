/**
 * Consecutive-failure accounting shared by every periodic job (vault-sync,
 * backup). The decision is pure; persisting the counter and writing the
 * notification file is an adapter's job (see @sdw/infra failure-tracker).
 */

/** Consecutive failures before a notification is written. */
export const DEFAULT_FAILURE_THRESHOLD = 3;

/** Whether the failure count has reached the point where the user should be told. */
export function reachedFailureThreshold(
  consecutiveFailures: number,
  threshold: number = DEFAULT_FAILURE_THRESHOLD,
): boolean {
  return consecutiveFailures >= threshold;
}

/** The side of a periodic job that reports how it is doing. */
export interface FailureTracker {
  /** Consecutive failures recorded so far (0 after a success). */
  consecutiveFailures(): number;
  recordSuccess(): void;
  /** Counts one more failure and, at the threshold, writes the notification. */
  recordFailure(): { count: number; notified: boolean };
}
