//! HolderControl adapter — a port of packages/infra/src/holders.ts. Stops everything that may hold
//! fds or inotify watches on the plaintext mount. Desktops first (XFCE is the primary HOME-fd
//! holder), then the backup watcher.
//!
//! The TS keeps module-level state of what the first `stop_all` found running, so a forced retry's
//! second `stop_all` does not clobber it; here that state lives in the instance (lock-vault reuses
//! one instance across the attempt, matching the TS lifetime).

use crate::cmd;
use crate::desktop::{
    installed_stacks, is_desktop_running, start_desktop, stop_all_desktops, target_name,
};
use sdw_core::desktop::domain::DesktopTarget;
use sdw_core::vault::ports::HolderControl;
use std::cell::RefCell;

fn backup_watcher_running() -> bool {
    let (_, out) = cmd::capture("supervisorctl", &["status", "backup-watcher"]);
    out.contains("RUNNING")
}

struct Stopped {
    desktops: Vec<DesktopTarget>,
    backup: bool,
}

#[derive(Default)]
pub struct SupervisorHolders {
    /// What the first `stop_all` of a lock attempt found running; kept for `restore`.
    stopped: RefCell<Option<Stopped>>,
}

impl SupervisorHolders {
    pub fn new() -> Self {
        Self::default()
    }
}

impl HolderControl for SupervisorHolders {
    fn stop_all(&self) {
        // The second call of a forced retry sees everything stopped already; the first one knows.
        if self.stopped.borrow().is_none() {
            let desktops = installed_stacks()
                .into_iter()
                .filter(|t| is_desktop_running(*t))
                .collect();
            *self.stopped.borrow_mut() = Some(Stopped {
                desktops,
                backup: backup_watcher_running(),
            });
        }
        stop_all_desktops();
        cmd::ok("supervisorctl", &["stop", "backup-watcher"]);
    }

    fn restore(&self) -> Vec<String> {
        let was = match self.stopped.borrow_mut().take() {
            Some(w) => w,
            None => return Vec::new(),
        };
        let mut started = Vec::new();
        if was.backup {
            cmd::ok("supervisorctl", &["start", "backup-watcher"]);
            started.push("backup watcher".to_string());
        }
        for target in was.desktops {
            start_desktop(Some(target));
            started.push(format!("desktop [{}]", target_name(target)));
        }
        started
    }
}
