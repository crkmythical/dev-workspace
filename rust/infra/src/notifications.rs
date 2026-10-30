//! Notes for the user ("backup is failing", "egress is degraded") as plain files — a port of
//! packages/infra/src/notifications.ts.
//!
//! They live inside the vault (NOTIFICATIONS_DIR), readable while unlocked and absent otherwise.
//! A write is REFUSED while nothing is mounted at the workspace root: the directory would then be
//! an ordinary one on the container's unencrypted filesystem and the note would land in the clear
//! (and be hidden under the mount once the vault comes up).

use sdw_core::shared::layout::WORKSPACE_MOUNT;
use std::fs;
use std::path::Path;

/// A directory on a different device than its parent is a mount point.
pub fn is_mount_point(dir: &str) -> bool {
    use std::os::unix::fs::MetadataExt;
    let here = match fs::metadata(dir) {
        Ok(m) => m.dev(),
        Err(_) => return false,
    };
    let parent = Path::new(dir).parent().unwrap_or_else(|| Path::new("/"));
    match fs::metadata(parent) {
        Ok(m) => here != m.dev(),
        Err(_) => false,
    }
}

/// Whether a note under `root` must be refused because `root` is not a live mount.
fn blocked(file: &str, root: &str, is_mounted: &dyn Fn(&str) -> bool) -> bool {
    file.starts_with(&format!("{root}/")) && !is_mounted(root)
}

/// Write a note. False when there was nowhere safe to write it (not fatal).
pub fn write_note(file: &str, body: &str) -> bool {
    write_note_guarded(file, body, WORKSPACE_MOUNT, &is_mount_point)
}

/// Remove a note, if there is one.
pub fn remove_note(file: &str) {
    remove_note_guarded(file, WORKSPACE_MOUNT, &is_mount_point)
}

fn write_note_guarded(
    file: &str,
    body: &str,
    root: &str,
    is_mounted: &dyn Fn(&str) -> bool,
) -> bool {
    if blocked(file, root, is_mounted) {
        return false;
    }
    if let Some(dir) = Path::new(file).parent() {
        let _ = fs::create_dir_all(dir);
    }
    fs::write(file, body).is_ok()
}

fn remove_note_guarded(file: &str, root: &str, is_mounted: &dyn Fn(&str) -> bool) {
    if blocked(file, root, is_mounted) {
        return;
    }
    let _ = fs::remove_file(file);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_under_the_root_is_blocked_only_while_unmounted() {
        let mounted = |_: &str| true;
        let unmounted = |_: &str| false;
        let note = "/workspace/.notifications/egress-degraded.md";
        assert!(blocked(note, "/workspace", &unmounted));
        assert!(!blocked(note, "/workspace", &mounted));
    }

    #[test]
    fn notes_outside_the_root_are_never_blocked() {
        let unmounted = |_: &str| false;
        // The egress STATUS file lives under /var/run, not the vault: always writable.
        assert!(!blocked("/var/run/egress-status", "/workspace", &unmounted));
        // Exactly the root (no trailing slash) is not "under" it.
        assert!(!blocked("/workspace", "/workspace", &unmounted));
    }
}
