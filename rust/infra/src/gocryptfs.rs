//! gocryptfs adapter: the VolumeDriver port plus the shell primitives behind it — a port of
//! packages/infra/src/gocryptfs.ts. The passphrase prompt stays in the CLI.
//!
//! The pure argv builder is unit-tested; the FUSE mount/unmount and the /proc-walking kill
//! escalation run only in the container and are validated by tests/smoke.

use crate::cmd;
use sdw_core::vault::destruct::DESTRUCT_COMPLETED_PATH;
use sdw_core::vault::domain::{killable, lineage, parse_fuser_pids, VaultSpec, VaultState};
use sdw_core::vault::ports::{MountOutcome, UnmountMode, VolumeDriver};
use std::fs;
use std::path::Path;

/// Check if a path is a FUSE mountpoint.
pub fn is_mounted(path: &str) -> bool {
    cmd::ok("mountpoint", &["-q", path])
}

/// Check if a vault has been initialized (gocryptfs.conf exists).
pub fn is_initialized(cipher_dir: &str) -> bool {
    Path::new(&format!("{cipher_dir}/gocryptfs.conf")).exists()
}

/// Ensure both cipher and mount directories exist.
pub fn ensure_dirs(cipher_dir: &str, mount_point: &str) {
    let _ = fs::create_dir_all(cipher_dir);
    let _ = fs::create_dir_all(mount_point);
}

/// Initialize a gocryptfs vault. With a passphrase it is piped (twice, init confirmation); without
/// one gocryptfs prompts on the inherited TTY. Returns true on success.
pub fn init_vault(cipher_dir: &str, passphrase: Option<&str>) -> bool {
    match passphrase {
        Some(p) => {
            cmd::run_stdin(
                "gocryptfs",
                &["-init", "-q", cipher_dir],
                &format!("{p}\n{p}\n"),
            )
            .0
        }
        None => cmd::inherit("gocryptfs", &["-init", cipher_dir]) == 0,
    }
}

/// The gocryptfs command line for a mount. Pure, so it can be tested without FUSE.
pub fn gocryptfs_mount_args(cipher_dir: &str, mount_point: &str, allow_other: bool) -> Vec<String> {
    let mut args = vec![
        "gocryptfs".to_string(),
        "-q".to_string(),
        "-nonempty".to_string(),
    ];
    if allow_other {
        args.push("-allow_other".to_string());
    }
    args.push(cipher_dir.to_string());
    args.push(mount_point.to_string());
    args
}

/// Mount a gocryptfs vault, piping the passphrase. Returns { ok, detail: gocryptfs's stderr }.
pub fn mount_vault(
    cipher_dir: &str,
    mount_point: &str,
    passphrase: &str,
    allow_other: bool,
) -> MountOutcome {
    let _ = fs::create_dir_all(mount_point);
    let args = gocryptfs_mount_args(cipher_dir, mount_point, allow_other);
    let argv: Vec<&str> = args[1..].iter().map(String::as_str).collect();
    let (ok, stderr) = cmd::run_stdin("gocryptfs", &argv, &format!("{passphrase}\n"));
    MountOutcome {
        ok,
        detail: Some(stderr),
    }
}

/// Parent of `pid` per /proc, or None once the process is gone.
fn parent_of(pid: u32) -> Option<u32> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // "pid (comm) state ppid ...": comm may hold spaces and parens, so cut after the last ")".
    let rest = stat.get(stat.rfind(')')? + 2..)?;
    rest.split(' ').nth(1)?.parse::<u32>().ok()
}

/// Leave the mount if this process's working directory is inside it: left in place it is a holder
/// of its own, and once detached `getcwd` fails so every later subprocess dies with ENOENT.
pub fn step_out_of(mount_point: &str) {
    if let Ok(cwd) = std::env::current_dir() {
        let cwd = cwd.to_string_lossy().into_owned();
        if cwd != mount_point && !cwd.starts_with(&format!("{mount_point}/")) {
            return;
        }
    }
    let _ = std::env::set_current_dir("/");
}

#[derive(Clone, Copy)]
enum Scope {
    Directory,
    Filesystem,
}

/// SIGKILL the processes using a mount, except this one and everything above it (core `killable`).
/// Needs `fuser` (psmisc); without it nothing is found and nothing is killed.
fn kill_holders(mount_point: &str, scope: Scope, report: &dyn Fn(&str)) {
    let (_, stdout) = match scope {
        Scope::Filesystem => cmd::capture("fuser", &["-m", mount_point]),
        Scope::Directory => cmd::capture("fuser", &[mount_point]),
    };
    let victims = killable(
        &parse_fuser_pids(&stdout),
        &lineage(std::process::id(), parent_of),
    );
    for pid in &victims {
        // Ignore failures: the process may already be gone. `kill` from procps is in the image.
        cmd::ok("kill", &["-KILL", &pid.to_string()]);
    }
    if !victims.is_empty() {
        let pids = victims
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        report(&format!(
            "  killed {} process(es) using {mount_point}: {pids}",
            victims.len()
        ));
    }
}

/// Release a FUSE mount. Returns true when the mount point is free (including when it was not
/// mounted). graceful = fusermount -u; lazy = fusermount -uz; escalate = graceful, then kill users
/// and retry, then lazy (never the calling process or its ancestors).
pub fn unmount_vault(mount_point: &str, mode: UnmountMode, report: &dyn Fn(&str)) -> bool {
    if !is_mounted(mount_point) {
        return true;
    }
    step_out_of(mount_point);

    let attempt = |lazy: bool| -> bool {
        if lazy {
            cmd::ok("fusermount", &["-uz", mount_point])
        } else {
            cmd::ok("fusermount", &["-u", mount_point])
        }
    };

    if mode == UnmountMode::Lazy {
        return attempt(true);
    }
    if attempt(false) {
        return true;
    }
    if mode == UnmountMode::Graceful {
        return false;
    }

    kill_holders(mount_point, Scope::Directory, report);
    std::thread::sleep(std::time::Duration::from_millis(500));
    if attempt(false) {
        return true;
    }

    kill_holders(mount_point, Scope::Filesystem, report);
    std::thread::sleep(std::time::Duration::from_millis(1000));
    if attempt(false) {
        return true;
    }

    attempt(true)
}

/// Clean up a stale FUSE mount left by a previous container lifecycle.
pub fn cleanup_stale_mount(mount_point: &str) {
    cmd::ok("fusermount", &["-uz", mount_point]);
}

/// VolumeDriver port backed by gocryptfs + FUSE.
pub struct GocryptfsVolume;

impl VolumeDriver for GocryptfsVolume {
    fn state(&self, spec: &VaultSpec) -> VaultState {
        if Path::new(DESTRUCT_COMPLETED_PATH).exists() {
            return VaultState::Destroyed;
        }
        if is_mounted(&spec.mount_point) {
            return VaultState::Unlocked;
        }
        if is_initialized(&spec.cipher_dir) {
            VaultState::Locked
        } else {
            VaultState::Uninitialized
        }
    }

    fn init(&self, spec: &VaultSpec, passphrase: &str) -> bool {
        // The TS treats an empty passphrase as "prompt interactively".
        let p = if passphrase.is_empty() {
            None
        } else {
            Some(passphrase)
        };
        init_vault(&spec.cipher_dir, p)
    }

    fn mount(&self, spec: &VaultSpec, passphrase: &str) -> MountOutcome {
        mount_vault(
            &spec.cipher_dir,
            &spec.mount_point,
            passphrase,
            spec.allow_other,
        )
    }

    fn unmount(
        &self,
        spec: &VaultSpec,
        mode: UnmountMode,
        report: &dyn Fn(&str),
    ) -> Result<bool, String> {
        Ok(unmount_vault(&spec.mount_point, mode, report))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_args_are_quiet_nonempty_and_order_preserved() {
        assert_eq!(
            gocryptfs_mount_args("/vault/cipher", "/workspace", false),
            [
                "gocryptfs",
                "-q",
                "-nonempty",
                "/vault/cipher",
                "/workspace"
            ]
        );
    }

    #[test]
    fn allow_other_is_inserted_before_the_paths() {
        assert_eq!(
            gocryptfs_mount_args("/pentest/cipher", "/pentest/rootfs", true),
            [
                "gocryptfs",
                "-q",
                "-nonempty",
                "-allow_other",
                "/pentest/cipher",
                "/pentest/rootfs"
            ]
        );
    }
}
