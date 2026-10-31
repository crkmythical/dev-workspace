//! FailureTracker on plain files — a port of packages/infra/src/failure-tracker.ts. One copy for
//! the two periodic jobs (vault-sync, backup):
//!   <state_dir>/last-success         ISO timestamp of the last success
//!   <state_dir>/consecutive-failures the counter (what `doctor` reads)
//!   <notifications_dir>/<file>       written once the threshold is reached
//! State is container-local (non-FUSE); losing it on restart is harmless.

use crate::notifications::{remove_note, write_note};
use crate::time::now_iso;
use sdw_core::shared::failure::{reached_failure_threshold, FailureTracker, RecordedFailure};
use sdw_core::shared::layout::NOTIFICATIONS_DIR;
use std::fs;

pub struct FileFailureTracker {
    state_dir: String,
    threshold: u32,
    notification_file: String,
    notification_body: fn(u32) -> String,
    notifications_dir: String,
}

impl FileFailureTracker {
    pub fn new(
        state_dir: &str,
        threshold: u32,
        notification_file: &str,
        notification_body: fn(u32) -> String,
    ) -> Self {
        let _ = fs::create_dir_all(state_dir);
        FileFailureTracker {
            state_dir: state_dir.to_string(),
            threshold,
            notification_file: notification_file.to_string(),
            notification_body,
            notifications_dir: NOTIFICATIONS_DIR.to_string(),
        }
    }

    fn count_file(&self) -> String {
        format!("{}/consecutive-failures", self.state_dir)
    }
    fn success_file(&self) -> String {
        format!("{}/last-success", self.state_dir)
    }
    fn note_path(&self) -> String {
        format!("{}/{}", self.notifications_dir, self.notification_file)
    }
    fn read(&self) -> u32 {
        fs::read_to_string(self.count_file())
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0)
    }
}

impl FailureTracker for FileFailureTracker {
    fn consecutive_failures(&self) -> u32 {
        self.read()
    }

    fn record_success(&mut self) {
        let _ = fs::write(self.success_file(), now_iso());
        let _ = fs::write(self.count_file(), "0");
        // A job that works again must not keep telling the user it is failing.
        remove_note(&self.note_path());
    }

    fn record_failure(&mut self) -> RecordedFailure {
        let count = self.read() + 1;
        let _ = fs::write(self.count_file(), count.to_string());
        if !reached_failure_threshold(count, self.threshold) {
            return RecordedFailure {
                count,
                notified: false,
            };
        }
        // With the vault locked there is nowhere safe to write; that is not fatal.
        let notified = write_note(&self.note_path(), &(self.notification_body)(count));
        RecordedFailure { count, notified }
    }
}
