pub mod backup_init;
pub mod backup_watcher;
pub mod desktop_install;
pub mod desktop_start;
pub mod desktop_stop;
pub mod destruct_passphrase;
pub mod doctor;
pub mod init_pentest;
pub mod init_vault;
pub mod lock_pentest;
pub mod lock_vault;
pub mod pentest_enter;
pub mod pull;
pub mod push;
pub mod self_destruct;
pub mod setup;
pub mod unlock_pentest;
pub mod unlock_vault;
pub mod vault_sync;

/// The real system environment getter the commands pass to core's `load_config`.
pub(crate) fn system_env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}
