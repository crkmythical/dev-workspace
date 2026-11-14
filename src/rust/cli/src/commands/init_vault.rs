//! init-vault — one-time gocryptfs vault initialization. A port of packages/cli/src/init-vault.ts.
//! Idempotent: an existing gocryptfs.conf means "already initialized".

use crate::commands::system_env;
use sdw_core::vault::domain::{decide, Decision, VaultCommand, VaultState};
use sdw_core::vault::domain::{VAULT_GITATTRIBUTES, VAULT_GITIGNORE};
use sdw_core::vault::layout::VAULT_CIPHER_DIR;
use sdw_core::workspace::config::load_config;
use sdw_infra::cmd;
use sdw_infra::gocryptfs::is_initialized;
use std::fs;

pub fn run(_args: &[String]) -> i32 {
    let state = if is_initialized(VAULT_CIPHER_DIR) {
        VaultState::Locked
    } else {
        VaultState::Uninitialized
    };
    if matches!(decide(state, &VaultCommand::Init), Decision::Noop(_)) {
        println!("Vault already initialized.");
        return 0;
    }

    println!("=== Vault Initialization ===\n");
    println!("Choose a strong passphrase (16+ chars, mix case/numbers/symbols).");
    println!("Store it in a password manager immediately.\n");

    // gocryptfs prompts for the passphrase on the inherited TTY.
    if !sdw_infra::gocryptfs::init_vault(VAULT_CIPHER_DIR, None) {
        eprintln!("gocryptfs init failed.");
        return 1;
    }

    let config = load_config(system_env);
    let email = config.git.user_email.as_deref().unwrap_or("vault@local");
    let name = config.git.user_name.as_deref().unwrap_or("Vault");
    cmd::ok("git", &["config", "--global", "user.email", email]);
    cmd::ok("git", &["config", "--global", "user.name", name]);
    cmd::ok("git", &["-C", VAULT_CIPHER_DIR, "init", "-b", "main"]);

    let _ = fs::write(format!("{VAULT_CIPHER_DIR}/.gitignore"), VAULT_GITIGNORE);
    let _ = fs::write(
        format!("{VAULT_CIPHER_DIR}/.gitattributes"),
        VAULT_GITATTRIBUTES,
    );

    cmd::ok("git", &["-C", VAULT_CIPHER_DIR, "add", "-A"]);
    cmd::ok(
        "git",
        &["-C", VAULT_CIPHER_DIR, "commit", "-q", "-m", "init vault"],
    );

    match config.git.vault_repo {
        Some(repo) => {
            cmd::ok(
                "git",
                &["-C", VAULT_CIPHER_DIR, "remote", "add", "origin", &repo],
            );
            cmd::ok(
                "git",
                &["-C", VAULT_CIPHER_DIR, "push", "-u", "origin", "main"],
            );
            println!("Pushed to {repo}");
        }
        None => {
            println!("No VAULT_GIT_REPO configured — skipping remote push.");
            println!("Set VAULT_GIT_REPO in .env to enable auto-sync later.");
        }
    }

    println!("\n✓ Vault initialized. Run 'unlock-vault' to mount.\n");
    println!("⚠ SAVE YOUR PASSPHRASE IN A PASSWORD MANAGER NOW.");
    0
}
