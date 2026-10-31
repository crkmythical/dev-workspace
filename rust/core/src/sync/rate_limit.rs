//! Fixed-window rate limiter with a per-key budget AND a global backstop — a port
//! of packages/core/src/shared/rate-limit.ts.
//!
//! The self-destruct endpoint derives its per-key value from a client-influenced
//! header (CF-Connecting-IP / X-Forwarded-For). A per-key budget alone can be
//! defeated by rotating that header, so a GLOBAL window caps total attempts
//! across ALL keys. The global cap cannot be bypassed by header spoofing; a flood
//! of spoofed keys instead exhausts it and blocks everyone for the rest of the
//! window, which for an irreversible action fails in the safe direction (the
//! local `self-destruct` CLI path is unaffected).

use std::collections::HashMap;

pub struct RateLimitConfig {
    pub window_ms: i64,
    pub per_key_max: u64,
    pub global_max: u64,
}

struct WindowState {
    count: u64,
    reset_at: i64,
}

pub struct RateLimiter {
    per_key: HashMap<String, WindowState>,
    global: Option<WindowState>,
    config: RateLimitConfig,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            per_key: HashMap::new(),
            global: None,
            config,
        }
    }

    /// Record an attempt for `key` at `now_ms`. Returns true if the attempt
    /// should be REJECTED (over the per-key or global budget). The order matches
    /// the TS original: the global backstop is counted first, on every call.
    pub fn check(&mut self, key: &str, now_ms: i64) -> bool {
        let (window_ms, global_max, per_key_max) = (
            self.config.window_ms,
            self.config.global_max,
            self.config.per_key_max,
        );

        match &mut self.global {
            Some(g) if now_ms <= g.reset_at => {
                g.count += 1;
                if g.count > global_max {
                    return true;
                }
            }
            _ => {
                self.global = Some(WindowState {
                    count: 1,
                    reset_at: now_ms + window_ms,
                });
            }
        }

        match self.per_key.get_mut(key) {
            Some(e) if now_ms <= e.reset_at => {
                e.count += 1;
                e.count > per_key_max
            }
            _ => {
                self.per_key.insert(
                    key.to_string(),
                    WindowState {
                        count: 1,
                        reset_at: now_ms + window_ms,
                    },
                );
                false
            }
        }
    }

    /// Drop all state.
    pub fn reset(&mut self) {
        self.per_key.clear();
        self.global = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter(per_key_max: u64, global_max: u64) -> RateLimiter {
        RateLimiter::new(RateLimitConfig {
            window_ms: 60_000,
            per_key_max,
            global_max,
        })
    }

    #[test]
    fn per_key_budget_then_reject() {
        let mut l = limiter(3, 1000);
        // 3 attempts allowed, the 4th rejected — all at the same instant.
        assert!(!l.check("a", 0));
        assert!(!l.check("a", 0));
        assert!(!l.check("a", 0));
        assert!(l.check("a", 0));
    }

    #[test]
    fn per_key_window_resets() {
        let mut l = limiter(3, 1000);
        for _ in 0..4 {
            l.check("a", 0);
        }
        assert!(l.check("a", 0), "still over budget inside the window");
        // A new window: the budget is fresh.
        assert!(!l.check("a", 60_001));
    }

    #[test]
    fn global_backstop_caps_spoofed_keys() {
        // Per-key budget is generous; the global cap is 10. Each request uses a
        // brand-new key, so only the global window can stop them.
        let mut l = limiter(1000, 10);
        for i in 0..10 {
            assert!(
                !l.check(&format!("key-{i}"), 0),
                "attempt {i} within global budget"
            );
        }
        assert!(
            l.check("key-11", 0),
            "the 11th distinct key trips the global cap"
        );
    }

    #[test]
    fn reset_clears_everything() {
        let mut l = limiter(1, 1);
        assert!(!l.check("a", 0));
        assert!(l.check("a", 0));
        l.reset();
        assert!(!l.check("a", 0), "after reset the budget is fresh");
    }
}
