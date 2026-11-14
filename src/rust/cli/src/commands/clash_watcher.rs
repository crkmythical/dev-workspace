//! `sdw clash-watcher` — what supervisord's [program:clash-watcher] runs: if Clash is not running,
//! start it, every few seconds, for as long as the container lives. Replaces clash-watcher.sh, so
//! that the boot, `setup`, `proxy-only` and this loop all start Clash the same way (the clash user,
//! its group, no supplementary groups) and by the same rule (only from a config we rendered).

use sdw_core::egress::layout::CLASH_USER;
use sdw_infra::clash::config_is_ready;
use sdw_infra::egress::{clash_command, clash_running, resolve_uid};
use std::process::{Child, Stdio};
use std::time::Duration;

const INTERVAL: Duration = Duration::from_secs(5);

pub fn run(_args: &[String]) -> i32 {
    let mut child: Option<Child> = None;
    let mut warned = false;
    loop {
        // Collect the exit of a Clash this loop started, or it stays a zombie.
        if let Some(c) = child.as_mut() {
            if !matches!(c.try_wait(), Ok(None)) {
                child = None;
            }
        }
        if !clash_running() {
            if config_is_ready() {
                warned = false;
                let mut command = clash_command(resolve_uid(CLASH_USER).is_some());
                command
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                match command.spawn() {
                    Ok(started) => child = Some(started),
                    Err(e) => eprintln!("clash-watcher: cannot start Clash: {e}"),
                }
            } else if !warned {
                warned = true;
                eprintln!(
                    "clash-watcher: Clash is not running and there is no rendered config; not \
                     starting it"
                );
            }
        }
        std::thread::sleep(INTERVAL);
    }
}
