//! gocryptfs adapter: the VolumeDriver port plus the shell primitives behind it — a port of
//! packages/infra/src/gocryptfs.ts. The passphrase prompt stays in the CLI.
//!
//! The pure argv builder is unit-tested; the FUSE mount/unmount and the /proc-walking kill
//! escalation run only in the container and are validated by tests/smoke.

use crate::cmd;
use sdw_core::vault::destruct::DESTRUCT_COMPLETED_PATH;
use sdw_core::vault::domain::{
    killable, lineage, parse_fuser_pids, socket_inodes_under_mount, VaultSpec, VaultState,
};
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

/// scrypt cost of a NEW vault's key wrap, as log2(N): N = 2^18 where gocryptfs defaults to 2^16.
/// The passphrase-wrapped master key sits in `gocryptfs.conf`, which is replicated with the
/// ciphertext to the git remote, so whoever gets that file can guess the passphrase offline. Four
/// times the work per guess costs an unlock about 0.4 s and 256 MiB of memory for that moment.
/// An existing vault keeps its own cost; `gocryptfs -passwd -scryptn 18 /vault/cipher` raises it.
pub const SCRYPT_LOG_N: u8 = 18;
// Weaker than gocryptfs's own default (16) would be a regression: refuse to compile it.
const _: () = assert!(SCRYPT_LOG_N > 16);

/// The `gocryptfs -init` command line (without the program name). Pure, for testing.
pub fn gocryptfs_init_args(cipher_dir: &str, quiet: bool) -> Vec<String> {
    let mut args = vec!["-init".to_string()];
    if quiet {
        args.push("-q".to_string());
    }
    args.push("-scryptn".to_string());
    args.push(SCRYPT_LOG_N.to_string());
    args.push(cipher_dir.to_string());
    args
}

/// Initialize a gocryptfs vault. With a passphrase it is piped (twice, init confirmation); without
/// one gocryptfs prompts on the inherited TTY. Returns true on success.
pub fn init_vault(cipher_dir: &str, passphrase: Option<&str>) -> bool {
    match passphrase {
        Some(p) => {
            let args = gocryptfs_init_args(cipher_dir, true);
            let argv: Vec<&str> = args.iter().map(String::as_str).collect();
            cmd::run_stdin("gocryptfs", &argv, &format!("{p}\n{p}\n")).0
        }
        None => {
            let args = gocryptfs_init_args(cipher_dir, false);
            let argv: Vec<&str> = args.iter().map(String::as_str).collect();
            cmd::inherit("gocryptfs", &argv) == 0
        }
    }
}

/// The gocryptfs command line for a mount. Pure, so it can be tested without FUSE. `owner` makes
/// every file show as owned by that uid:gid (`-force_owner`): the volume is mounted by root, and
/// its files belong to the unprivileged workload user.
pub fn gocryptfs_mount_args(
    cipher_dir: &str,
    mount_point: &str,
    allow_other: bool,
    owner: Option<(u32, u32)>,
) -> Vec<String> {
    let mut args = vec![
        "gocryptfs".to_string(),
        "-q".to_string(),
        "-nonempty".to_string(),
    ];
    if allow_other {
        args.push("-allow_other".to_string());
    }
    if let Some((uid, gid)) = owner {
        args.push("-force_owner".to_string());
        args.push(format!("{uid}:{gid}"));
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
    owner: Option<(u32, u32)>,
) -> MountOutcome {
    let _ = fs::create_dir_all(mount_point);
    let args = gocryptfs_mount_args(cipher_dir, mount_point, allow_other, owner);
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
/// PIDs holding a bound/listening unix socket whose path is at or under `mount_point` — holders
/// `fuser` does not report (a bound socket pins the mount without an open fd it sees). Reads the
/// socket inodes from /proc/net/unix (core selects the ones under the mount), then maps each to its
/// owner by the `socket:[inode]` symlink in /proc/<pid>/fd. Empty when /proc is unavailable.
fn socket_holder_pids(mount_point: &str) -> Vec<u32> {
    let Ok(content) = fs::read_to_string("/proc/net/unix") else {
        return Vec::new();
    };
    let inodes = socket_inodes_under_mount(&content, mount_point);
    if inodes.is_empty() {
        return Vec::new();
    }
    let wanted: std::collections::HashSet<String> =
        inodes.iter().map(|i| format!("socket:[{i}]")).collect();
    let mut pids = Vec::new();
    let Ok(entries) = fs::read_dir("/proc") else {
        return pids;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(fds) = fs::read_dir(format!("/proc/{pid}/fd")) else {
            continue;
        };
        for fd in fds.flatten() {
            if let Ok(target) = fs::read_link(fd.path()) {
                if wanted.contains(target.to_string_lossy().as_ref()) {
                    pids.push(pid);
                    break;
                }
            }
        }
    }
    pids
}

fn kill_holders(mount_point: &str, scope: Scope, report: &dyn Fn(&str)) {
    let (_, stdout) = match scope {
        Scope::Filesystem => cmd::capture("fuser", &["-m", mount_point]),
        Scope::Directory => cmd::capture("fuser", &[mount_point]),
    };
    let mut holders = parse_fuser_pids(&stdout);
    if let Scope::Filesystem = scope {
        // fuser misses a process that only holds a bound/listening unix socket under the mount; add
        // those so a clean unmount succeeds instead of falling back to lazy. killable drops dupes.
        holders.extend(socket_holder_pids(mount_point));
    }
    let victims = killable(&holders, &lineage(std::process::id(), parent_of));
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
            spec.owner,
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
            gocryptfs_mount_args("/vault/cipher", "/workspace", false, None),
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
    fn a_new_vault_gets_a_stronger_key_wrap_than_gocryptfs_defaults_to() {
        // The default is 16; 18 is four times the work per passphrase guess.
        assert_eq!(
            gocryptfs_init_args("/vault/cipher", true),
            ["-init", "-q", "-scryptn", "18", "/vault/cipher"]
        );
        // The interactive form differs only in letting gocryptfs talk.
        assert_eq!(
            gocryptfs_init_args("/pentest/cipher", false),
            ["-init", "-scryptn", "18", "/pentest/cipher"]
        );
    }

    #[test]
    fn a_forced_owner_follows_allow_other() {
        assert_eq!(
            gocryptfs_mount_args("/vault/cipher", "/workspace", true, Some((1000, 1000))),
            [
                "gocryptfs",
                "-q",
                "-nonempty",
                "-allow_other",
                "-force_owner",
                "1000:1000",
                "/vault/cipher",
                "/workspace"
            ]
        );
    }

    #[test]
    fn allow_other_is_inserted_before_the_paths() {
        assert_eq!(
            gocryptfs_mount_args("/pentest/cipher", "/pentest/rootfs", true, None),
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
