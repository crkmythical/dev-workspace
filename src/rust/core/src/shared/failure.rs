//! Consecutive-failure accounting shared by the periodic jobs (vault-sync, backup). The decision
//! is pure; persisting the counter and writing the notification is an adapter's job (infra). A
//! port of packages/core/src/shared/failure.ts.

/// Consecutive failures before a notification is written.
pub const DEFAULT_FAILURE_THRESHOLD: u32 = 3;

/// Whether the failure count has reached the point where the user should be told.
pub fn reached_failure_threshold(consecutive_failures: u32, threshold: u32) -> bool {
    consecutive_failures >= threshold
}

/// What `record_failure` reports back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordedFailure {
    pub count: u32,
    pub notified: bool,
}

/// The side of a periodic job that reports how it is doing (implemented by an infra adapter).
pub trait FailureTracker {
    /// Consecutive failures recorded so far (0 after a success).
    fn consecutive_failures(&self) -> u32;
    fn record_success(&mut self);
    /// Counts one more failure and, at the threshold, writes the notification.
    fn record_failure(&mut self) -> RecordedFailure;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold() {
        assert!(!reached_failure_threshold(0, DEFAULT_FAILURE_THRESHOLD));
        assert!(!reached_failure_threshold(2, DEFAULT_FAILURE_THRESHOLD));
        assert!(reached_failure_threshold(3, DEFAULT_FAILURE_THRESHOLD));
        assert!(reached_failure_threshold(9, DEFAULT_FAILURE_THRESHOLD));
        // A custom threshold is honoured.
        assert!(reached_failure_threshold(1, 1));
        assert!(!reached_failure_threshold(1, 2));
    }
}
