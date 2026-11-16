//! Starting processes as the workload user, from root (see `sdw_core::workspace::workload`).
//!
//! Dropping privileges here means: the user's uid AND gid, no supplementary groups (std clears them
//! when a root process sets a uid and gives none), and an environment built from an allowlist
//! instead of inherited, because the container's own environment holds the secrets.

use sdw_core::egress::allowlist::AllowList;
use sdw_core::workspace::features::parse_feature_flags;
use sdw_core::workspace::workload::{
    parse_requested_names, workload_env, WORKLOAD_GID, WORKLOAD_UID,
};
use std::fs;
use std::os::unix::fs::chown;
use std::path::Path;
use std::process::Command;

/// Whether this process runs as root (effective uid 0).
pub fn is_root() -> bool {
    rustix::process::geteuid().is_root()
}

/// The names the user listed in `WORKLOAD_ENV` to pass on to the workload.
pub fn requested_env_names() -> Vec<String> {
    parse_requested_names(&std::env::var("WORKLOAD_ENV").unwrap_or_default())
}

/// The networks the egress firewall lets the workload reach without Clash (`EGRESS_ALLOW_CIDRS`),
/// as `no_proxy` entries. Empty when the firewall is off or nothing is allowed. The boot has
/// already refused a malformed list, so one that does not parse here is simply left out.
fn direct_networks() -> Vec<String> {
    if !parse_feature_flags(|name| std::env::var(name).ok()).egress_enforce {
        return Vec::new();
    }
    AllowList::parse(&std::env::var("EGRESS_ALLOW_CIDRS").unwrap_or_default())
        .map(|list| list.no_proxy_entries())
        .unwrap_or_default()
}

/// `program args...` as the workload user, with the allowlisted environment and `home` as HOME.
/// Returns the command and the requested variable names that were refused for being secrets.
/// The caller decides how to run it (exec, status, spawn).
pub fn workload_command(
    program: &str,
    args: &[String],
    home: &str,
    cwd: Option<&str>,
) -> (Command, Vec<String>) {
    let (env, refused) = workload_env(
        std::env::vars(),
        &requested_env_names(),
        home,
        &direct_networks(),
    );
    let mut command = Command::new(program);
    command
        .args(args)
        .env_clear()
        .envs(env)
        .uid(WORKLOAD_UID)
        .gid(WORKLOAD_GID)
        .current_dir(cwd.unwrap_or(home));
    (command, refused)
}

// `uid`/`gid` come from this extension trait.
use std::os::unix::process::CommandExt;

/// Run `program args...` to completion as the workload user in `cwd`; true when it exited 0. Its
/// output is discarded, like `cmd::ok`.
pub fn ok_as_workload(program: &str, args: &[&str], home: &str, cwd: &str) -> bool {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let (mut command, _) = workload_command(program, &args, home, Some(cwd));
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Give one path provisioned by root inside the mounted workspace vault to `dev`.
/// gocryptfs `force_owner` makes ownership look uniform on most hosts, but permission checks can
/// still observe root-owned directories on some distro/architecture combinations. Never follow a
/// symlink here: credential links intentionally point outside the path being provisioned.
pub fn own_path_by_workload(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    chown(path, Some(WORKLOAD_UID), Some(WORKLOAD_GID))
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn the_command_carries_the_clean_environment_and_the_workload_identity() {
        let (cmd, _) = workload_command(
            "code-server",
            &["--auth".into(), "none".into()],
            "/workspace/home",
            None,
        );
        assert_eq!(cmd.get_program(), "code-server");
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), ["--auth", "none"]);
        assert_eq!(
            cmd.get_current_dir(),
            Some(std::path::Path::new("/workspace/home"))
        );
        let envs: Vec<_> = cmd.get_envs().collect();
        let value = |name: &str| {
            envs.iter()
                .find(|(k, _)| *k == OsStr::new(name))
                .and_then(|(_, v)| *v)
        };
        assert_eq!(value("HOME"), Some(OsStr::new("/workspace/home")));
        assert_eq!(value("USER"), Some(OsStr::new("dev")));
        // The tests' own environment may hold anything; none of the workspace's secrets can pass.
        for name in [
            "PASSWORD",
            "RESTIC_PASSWORD",
            "CLASH_SUBSCRIPTION_URL",
            "DESKTOP_BCRYPT_HASH",
        ] {
            assert!(value(name).is_none(), "{name}");
        }
    }
}
