/** In-container service ports and system paths of the workspace itself. */

/** The single ingress: Caddy routes code-server, sync and the desktops behind it. */
export const CADDY_PORT = 8080;
export const CODE_SERVER_PORT = 8082;

export const CODE_SERVER_CONFIG_DIR = "/root/.config/code-server";
export const CODE_SERVER_CONFIG_PATH = `${CODE_SERVER_CONFIG_DIR}/config.yaml`;

export const SUPERVISORD_BIN = "/usr/bin/supervisord";
export const SUPERVISORD_CONF = "/etc/supervisor/supervisord.conf";
