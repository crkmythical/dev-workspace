//! `sdw sync-service` — run the sync API daemon (supervisord launches it in the container). Folds
//! the former standalone `sync-service` binary into the single `sdw` binary; the behaviour is that
//! crate's `run_blocking`, which serves /sync/api/* until the process is terminated.

pub fn run(_args: &[String]) -> i32 {
    sync_service::run_blocking();
    0
}
