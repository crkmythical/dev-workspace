//! Ring-buffer replay window — mirrors packages/core/src/replay-window.ts.
//! O(1) insert/lookup, fixed memory.

use std::collections::HashSet;

use super::protocol::{REPLAY_TOLERANCE_MS, REPLAY_WINDOW_SIZE};

pub struct ReplayWindow {
    buffer: Vec<Option<(String, i64)>>,
    head: usize,
    set: HashSet<String>,
    max_size: usize,
    tolerance_ms: i64,
}

impl ReplayWindow {
    pub fn new() -> Self {
        Self::with(REPLAY_WINDOW_SIZE, REPLAY_TOLERANCE_MS)
    }

    pub fn with(max_size: usize, tolerance_ms: i64) -> Self {
        Self {
            buffer: vec![None; max_size],
            head: 0,
            set: HashSet::new(),
            max_size,
            tolerance_ms,
        }
    }

    /// Returns true if the envelope is accepted, false on replay/out-of-window.
    pub fn check(&mut self, nonce_hex: &str, timestamp_ms: i64, now_ms: i64) -> bool {
        if (now_ms - timestamp_ms).abs() > self.tolerance_ms {
            return false;
        }
        if self.set.contains(nonce_hex) {
            return false;
        }
        if let Some((old, _)) = self.buffer[self.head].take() {
            self.set.remove(&old);
        }
        self.buffer[self.head] = Some((nonce_hex.to_string(), timestamp_ms));
        self.set.insert(nonce_hex.to_string());
        self.head = (self.head + 1) % self.max_size;
        true
    }

    pub fn reset(&mut self) {
        for slot in self.buffer.iter_mut() {
            *slot = None;
        }
        self.set.clear();
        self.head = 0;
    }
}

impl Default for ReplayWindow {
    fn default() -> Self {
        Self::new()
    }
}
