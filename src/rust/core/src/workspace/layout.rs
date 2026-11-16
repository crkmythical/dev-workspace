//! In-container service ports and system paths of the workspace itself — a port of
//! packages/core/src/workspace/layout.ts.

/// The single ingress: Caddy routes code-server, sync and the desktops behind it.
pub const CADDY_PORT: u16 = 8080;
pub const CODE_SERVER_PORT: u16 = 8082;
pub const EDITOR_SUITE_PATH: &str = "/etc/sdw-editor-suite";

/// Legacy images predate the marker and always contain code-server. New profile images write
/// `code-server` or `none` explicitly.
pub fn code_server_selected(raw: Option<&str>) -> bool {
    raw.is_none_or(|value| value.trim() == "code-server")
}

pub const SUPERVISORD_BIN: &str = "/usr/bin/supervisord";
pub const SUPERVISORD_CONF: &str = "/etc/supervisor/supervisord.conf";
/// Per-feature program definitions; the boot flips `autostart` in the ones that apply.
pub const SUPERVISOR_CONF_DIR: &str = "/etc/supervisor/conf.d";

/// The program-definition file the boot may enable, by name ("desktop" | "vnc" | "backup-watcher").
pub fn supervisor_conf(name: &str) -> String {
    format!("{SUPERVISOR_CONF_DIR}/{name}.conf")
}
