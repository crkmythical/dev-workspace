//! Vault-state broadcast — a port of the broadcast_state helper in
//! packages/infra/src/unlock-hooks.ts. Best-effort, fire-and-forget: it writes {"state":...} to
//! the state socket if anything is listening (nothing does today — lock/unlock is detected by
//! polling), so a failure changes nothing.

use crate::cmd;
use sdw_core::vault::layout::STATE_SOCKET_PATH;

pub fn broadcast_state(state: &str) {
    let json = format!("{{\"state\":\"{state}\"}}");
    cmd::run_stdin(
        "socat",
        &["-", &format!("UNIX-CONNECT:{STATE_SOCKET_PATH}")],
        &json,
    );
}
