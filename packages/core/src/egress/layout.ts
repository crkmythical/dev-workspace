/** Egress proxy (Clash/mihomo): ports and system paths. */

export const CLASH_HTTP_PORT = 7890;
export const CLASH_SOCKS_PORT = 7891;
export const CLASH_CONTROLLER_PORT = 9090;

export const CLASH_BIN = "/usr/bin/clash";
export const CLASH_DIR = "/etc/clash";
export const CLASH_CONFIG_PATH = `${CLASH_DIR}/config.yaml`;
export const CLASH_TEMPLATE_PATH = `${CLASH_DIR}/config.yaml.template`;
export const CLASH_PROVIDERS_DIR = `${CLASH_DIR}/providers`;
