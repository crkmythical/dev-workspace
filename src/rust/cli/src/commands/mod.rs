pub mod audit_log;
pub mod backup_init;
pub mod backup_watcher;
pub mod clash_dns;
pub mod clash_watcher;
pub mod desktop_install;
pub mod desktop_start;
pub mod desktop_stop;
pub mod destruct_passphrase;
pub mod doctor;
pub mod egress_monitor;
pub mod entrypoint;
pub mod env_example;
pub mod init_pentest;
pub mod init_vault;
pub mod lock_pentest;
pub mod lock_vault;
pub mod pentest_bootstrap;
pub mod pentest_enter;
pub mod proxy_only;
pub mod pull;
pub mod push;
pub mod self_destruct;
pub mod setup;
pub mod sync_service;
pub mod unlock_pentest;
pub mod unlock_vault;
pub mod vault_sync;
pub mod workload;

/// The real system environment getter the commands pass to core's `load_config`.
pub(crate) fn system_env(key: &str) -> Option<String> {
    // The secret variables (PASSWORD, RESTIC_PASSWORD, ...) may be given as `<NAME>_FILE` instead:
    // a Docker/Compose secret mounted under /run/secrets. Everything else is plain environment.
    sdw_infra::secrets::env_or_secret(key)
}
