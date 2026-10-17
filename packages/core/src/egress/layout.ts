/** Egress proxy (Clash/mihomo): ports, names and system paths. */
import { NOTIFICATIONS_DIR, RUN_DIR } from "../shared/layout.ts";

export const CLASH_HTTP_PORT = 7890;
export const CLASH_SOCKS_PORT = 7891;
export const CLASH_CONTROLLER_PORT = 9090;

export const CLASH_BIN = "/usr/bin/clash";
export const CLASH_DIR = "/etc/clash";
export const CLASH_CONFIG_PATH = `${CLASH_DIR}/config.yaml`;
export const CLASH_TEMPLATE_PATH = `${CLASH_DIR}/config.yaml.template`;
export const CLASH_PROVIDERS_DIR = `${CLASH_DIR}/providers`;

/** Names defined by config.yaml.template: the subscription provider and its url-test group. */
export const CLASH_PROVIDER_NAME = "subscription";
export const CLASH_AUTO_GROUP = "auto-select";

/** What the start-up probe fetches THROUGH the proxy to prove traffic gets out. */
export const EGRESS_PROBE_URL = "https://www.google.com";

/** The start-up observation, for `doctor` and anything else that wants it (container-local). */
export const EGRESS_STATUS_PATH = `${RUN_DIR}/egress-status`;
/** Written next to the other notifications while egress is not confirmed. */
export const EGRESS_NOTIFICATION_PATH = `${NOTIFICATIONS_DIR}/egress-degraded.md`;
