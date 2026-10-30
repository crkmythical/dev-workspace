//! Remote desktop: marker, HOME dirs, app dirs, displays and web roots for both stacks — a port of
//! packages/core/src/desktop/layout.ts. The default resolution and stream ports live in `domain`
//! where the topology uses them. Paths deriving from WORKSPACE_MOUNT are spelled as literals (see
//! crate::shared::layout).

/// Build-time marker written by the Dockerfile: "both" | "selkies" | "kasmvnc".
pub const DESKTOP_STACK_MARKER_PATH: &str = "/etc/sdw-desktop-stack";

pub const DESKTOP_CACHE_DIR: &str = "/tmp/.desktop-cache";
pub const DESKTOP_APPS_DIR: &str = "/opt/desktop-apps";
pub const DESKTOP_APPS_REGISTRY: &str = "/opt/desktop-apps/.registry.json";
/// Where installed desktop apps register their .desktop entries.
pub const DESKTOP_APPLICATIONS_DIR: &str = "/usr/share/applications";

// Selkies stack: H.264/WebCodecs, served at /desktop/.
pub const SELKIES_DISPLAY: &str = ":1";
/// `{WORKSPACE_MOUNT}/.desktop`.
pub const SELKIES_HOME: &str = "/workspace/.desktop";
/// Static Selkies web client, served by Caddy.
pub const SELKIES_WEB_ROOT: &str = "/usr/share/selkies/web";

// KasmVNC stack: integrated X + VNC server, served at /vnc/.
pub const VNC_DISPLAY: &str = ":2";
/// `{WORKSPACE_MOUNT}/.desktop-vnc`.
pub const VNC_HOME: &str = "/workspace/.desktop-vnc";
/// KasmVNC serves its own client from its own package.
pub const VNC_WEB_ROOT: &str = "/usr/share/kasmvnc/www";
