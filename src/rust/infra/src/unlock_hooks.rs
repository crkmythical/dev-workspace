//! Post-unlock hooks — a port of packages/infra/src/unlock-hooks.ts. The same implementations
//! serve the container's boot-time auto-unlock and the interactive `unlock-vault`; each root only
//! chooses WHICH hooks to run. Feature-gated hooks are idempotent "make the state match the flag":
//! enabled → provision, disabled → remove stale material.

use crate::cmd;
use crate::destruct_key::{derive_destruct_key, destruct_key_to_passphrase};
use crate::state::broadcast_state;
use sdw_core::shared::layout::{CREDENTIALS_DIR, WORKSPACE_MOUNT};
use sdw_core::sync::layout::SYNC_PASSPHRASE_PATH;
use sdw_core::vault::destruct::DESTRUCT_KEY_HASH_PATH;
use sdw_core::vault::ports::{UnlockContext, UnlockHook};
use sdw_core::workspace::features::FeatureFlags;
use sdw_core::workspace::workload::WORKLOAD_HOME;
use std::fs;
use std::path::Path;

type Announce = fn(&str);

fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/root".to_string())
}

/// Create the shared sync directory in the workspace vault.
struct EnsureSharedDir;
impl UnlockHook for EnsureSharedDir {
    fn name(&self) -> &str {
        "shared-dir"
    }
    fn run(&self, ctx: &UnlockContext<'_>) -> Result<(), String> {
        if ctx.spec.id == "workspace" {
            let _ = fs::create_dir_all(format!("{}/shared", ctx.spec.mount_point));
        }
        Ok(())
    }
}

/// The workload's home inside the vault: created, and given the shell start-up files a new user
/// gets, if they are not there. (The vault shows every file as the workload's, whoever creates it.)
struct PrepareWorkloadHome;
impl UnlockHook for PrepareWorkloadHome {
    fn name(&self) -> &str {
        "workload-home"
    }
    fn run(&self, ctx: &UnlockContext<'_>) -> Result<(), String> {
        if ctx.spec.id != "workspace" {
            return Ok(());
        }
        for dir in [".local/bin", ".config"] {
            fs::create_dir_all(format!("{WORKLOAD_HOME}/{dir}")).map_err(|e| e.to_string())?;
        }
        for name in [".bashrc", ".profile", ".bash_logout"] {
            let target = format!("{WORKLOAD_HOME}/{name}");
            let skeleton = format!("/etc/skel/{name}");
            if !Path::new(&target).exists() && Path::new(&skeleton).exists() {
                let _ = fs::copy(&skeleton, &target);
            }
        }
        Ok(())
    }
}

/// Link ~/.ssh to the credentials stored inside the vault, if any.
struct LinkSshCredentials {
    announce: Announce,
}
impl UnlockHook for LinkSshCredentials {
    fn name(&self) -> &str {
        "ssh-credentials"
    }
    fn run(&self, ctx: &UnlockContext<'_>) -> Result<(), String> {
        if ctx.spec.id != "workspace" {
            return Ok(());
        }
        let ssh_creds = format!("{CREDENTIALS_DIR}/ssh");
        if Path::new(&ssh_creds).exists() {
            // Root's (the ciphertext replication pushes over SSH) and the workload's own.
            for ssh_link in [format!("{}/.ssh", home()), format!("{WORKLOAD_HOME}/.ssh")] {
                cmd::ok("rm", &["-f", &ssh_link]);
                cmd::ok("ln", &["-sf", &ssh_creds, &ssh_link]);
            }
            (self.announce)("SSH credentials linked.");
        } else {
            (self.announce)(&format!(
                "No SSH credentials at {ssh_creds} — set up later."
            ));
        }
        Ok(())
    }
}

/// Point git's credential helper at the token stored inside the vault, if any.
struct GitCredentials {
    announce: Announce,
}
impl UnlockHook for GitCredentials {
    fn name(&self) -> &str {
        "git-credentials"
    }
    fn run(&self, ctx: &UnlockContext<'_>) -> Result<(), String> {
        if ctx.spec.id != "workspace" {
            return Ok(());
        }
        let token = format!("{CREDENTIALS_DIR}/github-token");
        if !Path::new(&token).exists() {
            return Ok(());
        }
        cmd::ok(
            "git",
            &[
                "config",
                "--system",
                "credential.helper",
                &format!("store --file={token}"),
            ],
        );
        (self.announce)("Git credential helper configured.");
        Ok(())
    }
}

/// SYNC_ENABLED: write the vault passphrase to the sync key source (a root-only file, removed again
/// when the vault locks). Disabled: remove a stale copy.
struct SyncCredential {
    enabled: bool,
    path: String,
    announce: Announce,
}
impl UnlockHook for SyncCredential {
    fn name(&self) -> &str {
        "sync-credential"
    }
    fn run(&self, ctx: &UnlockContext<'_>) -> Result<(), String> {
        if ctx.spec.id != "workspace" {
            return Ok(());
        }
        if !self.enabled {
            if Path::new(&self.path).exists() {
                let _ = fs::remove_file(&self.path);
                (self.announce)("Sync disabled: removed stale sync credential.");
            }
            return Ok(());
        }
        // Root-only and outside the vault (the workload owns every file inside it); see
        // core::sync::layout::SYNC_PASSPHRASE_PATH.
        crate::secrets::write_private_file(Path::new(&self.path), ctx.passphrase)
    }
}

/// REMOTE_DESTRUCT_ENABLED: derive the remote-destruct passphrase and store only its bcrypt hash;
/// announce the passphrase the first time the hash is made. Disabled: remove a stale hash.
struct RemoteDestructKey {
    enabled: bool,
    announce: Announce,
    hash_path: String,
    cost: u32,
}
impl UnlockHook for RemoteDestructKey {
    fn name(&self) -> &str {
        "remote-destruct-key"
    }
    fn run(&self, ctx: &UnlockContext<'_>) -> Result<(), String> {
        if ctx.spec.id != "workspace" {
            return Ok(());
        }
        if !self.enabled {
            if Path::new(&self.hash_path).exists() {
                let _ = fs::remove_file(&self.hash_path);
                (self.announce)("Remote self-destruct disabled: removed stale key hash.");
            }
            return Ok(());
        }
        let destruct_passphrase = destruct_key_to_passphrase(&derive_destruct_key(ctx.passphrase));
        let existed = Path::new(&self.hash_path).exists();
        let hash = bcrypt::hash(&destruct_passphrase, self.cost).map_err(|e| e.to_string())?;
        // The hash lets whoever holds it test guesses of the vault passphrase it derives from, so
        // it is root's alone (0600, in a 0700 directory), like the other secrets under /var/lib/sdw.
        crate::secrets::write_private_file(Path::new(&self.hash_path), &hash)?;
        if !existed {
            (self.announce)(&format!(
                "\nDestruction passphrase (save this): {destruct_passphrase}\n"
            ));
        }
        Ok(())
    }
}

/// Tell listeners on the state socket that the vault unlocked.
struct BroadcastStateHook;
impl UnlockHook for BroadcastStateHook {
    fn name(&self) -> &str {
        "broadcast-state"
    }
    fn run(&self, ctx: &UnlockContext<'_>) -> Result<(), String> {
        if ctx.spec.id == "workspace" {
            broadcast_state("unlocked");
        }
        Ok(())
    }
}

/// Install the user's mise-managed tools if the vault carries a .mise.toml.
struct MiseInstall;
impl UnlockHook for MiseInstall {
    fn name(&self) -> &str {
        "mise-install"
    }
    fn run(&self, ctx: &UnlockContext<'_>) -> Result<(), String> {
        if ctx.spec.id != "workspace" {
            return Ok(());
        }
        if Path::new(&format!("{WORKSPACE_MOUNT}/.mise.toml")).exists() {
            // The user's tools are installed by the user: into the workload's home, not root's.
            crate::workload::ok_as_workload("mise", &["install"], WORKLOAD_HOME, WORKSPACE_MOUNT);
        }
        Ok(())
    }
}

/// Start the backup watcher under supervisord (only when a restic repo is configured).
struct StartBackupWatcher {
    enabled: bool,
}
impl UnlockHook for StartBackupWatcher {
    fn name(&self) -> &str {
        "start-backup-watcher"
    }
    fn run(&self, ctx: &UnlockContext<'_>) -> Result<(), String> {
        if ctx.spec.id == "workspace" && self.enabled {
            cmd::ok("supervisorctl", &["start", "backup-watcher"]);
        }
        Ok(())
    }
}

/// Hooks every unlock of the workspace vault runs, at boot or from the command line.
pub fn workspace_unlock_hooks(flags: FeatureFlags, announce: Announce) -> Vec<Box<dyn UnlockHook>> {
    vec![
        Box::new(EnsureSharedDir),
        Box::new(PrepareWorkloadHome),
        Box::new(LinkSshCredentials { announce }),
        Box::new(GitCredentials { announce }),
        Box::new(SyncCredential {
            enabled: flags.sync,
            path: SYNC_PASSPHRASE_PATH.to_string(),
            announce,
        }),
        Box::new(RemoteDestructKey {
            enabled: flags.remote_destruct,
            announce,
            hash_path: DESTRUCT_KEY_HASH_PATH.to_string(),
            cost: 12,
        }),
    ]
}

/// Extra hooks for an interactive unlock only (they need a running supervisord, or are slow).
pub fn interactive_unlock_extras(backup_enabled: bool) -> Vec<Box<dyn UnlockHook>> {
    vec![
        Box::new(BroadcastStateHook),
        Box::new(MiseInstall),
        Box::new(StartBackupWatcher {
            enabled: backup_enabled,
        }),
    ]
}
