//! Egress proxy (Clash/mihomo): ports, names and system paths — a port of
//! packages/core/src/egress/layout.ts.

pub const CLASH_HTTP_PORT: u16 = 7890;
pub const CLASH_SOCKS_PORT: u16 = 7891;
pub const CLASH_CONTROLLER_PORT: u16 = 9090;

pub const CLASH_BIN: &str = "/usr/bin/clash";
pub const CLASH_DIR: &str = "/etc/clash";
pub const CLASH_CONFIG_PATH: &str = "/etc/clash/config.yaml";
pub const CLASH_TEMPLATE_PATH: &str = "/etc/clash/config.yaml.template";
pub const CLASH_PROVIDERS_DIR: &str = "/etc/clash/providers";

/// Names defined by config.yaml.template: the subscription provider and its url-test group.
pub const CLASH_PROVIDER_NAME: &str = "subscription";
pub const CLASH_AUTO_GROUP: &str = "auto-select";

/// What the start-up probe fetches THROUGH the proxy to prove traffic gets out.
pub const EGRESS_PROBE_URL: &str = "https://www.google.com";

/// The start-up observation, for `doctor` and anything else that wants it (container-local).
/// `{RUN_DIR}/egress-status`.
pub const EGRESS_STATUS_PATH: &str = "/var/run/egress-status";
/// Written next to the other notifications while egress is not confirmed.
/// `{NOTIFICATIONS_DIR}/egress-degraded.md`.
pub const EGRESS_NOTIFICATION_PATH: &str = "/workspace/.notifications/egress-degraded.md";
