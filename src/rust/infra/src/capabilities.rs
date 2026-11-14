//! The Linux capabilities this process holds (see `sdw_core::workspace::capabilities`).

use sdw_core::workspace::capabilities::effective_set;

/// The effective capability set of the current process, or None when `/proc` cannot be read.
pub fn effective_capabilities() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| effective_set(&status))
}
