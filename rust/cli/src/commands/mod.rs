pub mod backup_init;
pub mod destruct_passphrase;
pub mod lock_vault;
pub mod pull;
pub mod push;
pub mod self_destruct;

/// The real system environment getter the commands pass to core's `load_config`.
pub(crate) fn system_env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}
