//! Egress proxy (Clash/mihomo): ports, names and system paths — a port of
//! packages/core/src/egress/layout.ts.

pub const CLASH_HTTP_PORT: u16 = 7890;
pub const CLASH_SOCKS_PORT: u16 = 7891;
pub const CLASH_CONTROLLER_PORT: u16 = 9090;

pub const CLASH_BIN: &str = "/usr/bin/clash";
/// Dedicated system user Clash runs as, so the egress firewall can match its traffic by uid and
/// let ONLY Clash reach the network directly (everything else is dropped). Created in the image.
pub const CLASH_USER: &str = "clash";
/// Clash's working directory: a named volume, so everything in it outlives the container.
pub const CLASH_DIR: &str = "/etc/clash";
/// The rendered config. Holds the subscription URL and the controller secret, so it is mode 0600
/// and owned by the clash user; nothing else may read it.
pub const CLASH_CONFIG_PATH: &str = "/etc/clash/config.yaml";
/// The template the config is rendered from. It belongs to the IMAGE, so it lives outside the
/// volume: a file inside CLASH_DIR is copied out of the image only when the volume is first
/// created and is never updated again, which froze every installation on its first template.
pub const CLASH_TEMPLATE_PATH: &str = "/usr/share/sdw/clash/config.yaml.template";
pub const CLASH_PROVIDERS_DIR: &str = "/etc/clash/providers";
/// Runtime DNS override written by `set-clash-dns`. On the clash-config volume, so it survives a
/// restart/recreate and takes precedence over CLASH_DNS and auto-detection at the next render.
pub const CLASH_DNS_OVERRIDE_PATH: &str = "/etc/clash/dns.override";

/// Names defined by config.yaml.template: the subscription provider and its url-test group.
pub const CLASH_PROVIDER_NAME: &str = "subscription";
pub const CLASH_AUTO_GROUP: &str = "auto-select";

/// What the start-up probe fetches THROUGH the proxy to prove traffic gets out.
pub const EGRESS_PROBE_URL: &str = "https://www.google.com";

/// Written at boot while the egress firewall is on: `ssh` does not read the proxy variables, so
/// without it a Git remote over SSH would hit the firewall. See `firewall::ssh_proxy_config`.
pub const SSH_PROXY_CONF_PATH: &str = "/etc/ssh/ssh_config.d/10-sdw-egress.conf";

/// The start-up observation, for `doctor` and anything else that wants it (container-local).
/// `{RUN_DIR}/egress-status`.
pub const EGRESS_STATUS_PATH: &str = "/var/run/egress-status";
/// Written next to the other notifications while egress is not confirmed.
/// `{NOTIFICATIONS_DIR}/egress-degraded.md`.
pub const EGRESS_NOTIFICATION_PATH: &str = "/workspace/.notifications/egress-degraded.md";
