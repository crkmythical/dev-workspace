//! destruct-passphrase — print the remote self-destruct passphrase. A port of
//! packages/cli/src/destruct-passphrase.ts.
//!
//! The passphrase is a deterministic HKDF of the vault passphrase, so it can be re-derived at any
//! time instead of being saved from the one-time banner. The vault passphrase comes from $PASSWORD
//! if set, else is prompted / piped.

use crate::commands::system_env;
use crate::passphrase::read_passphrase;
use sdw_core::workspace::config::load_config;
use sdw_infra::destruct_key::{derive_destruct_key, destruct_key_to_passphrase};

pub fn run(_args: &[String]) -> i32 {
    let config = load_config(system_env);
    let passphrase = read_passphrase(config.password.as_deref(), "Vault passphrase: ");
    if passphrase.is_empty() {
        eprintln!("ERROR: Passphrase cannot be empty.");
        return 1;
    }

    println!(
        "{}",
        destruct_key_to_passphrase(&derive_destruct_key(&passphrase))
    );

    if !config.features.remote_destruct {
        eprintln!("Note: REMOTE_DESTRUCT_ENABLED is not set, so the remote endpoint is off.");
    }
    0
}
