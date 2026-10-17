/**
 * Remote desktop: ports, displays, HOME dirs and system paths for both stacks.
 *
 * Each stack gets an independent X display, WS port and HOME, so the two can run
 * in parallel behind one Caddy port without sharing session state.
 */
import { WORKSPACE_MOUNT } from "../shared/layout.ts";

export const DESKTOP_DEFAULT_RESOLUTION = "1920x1080";

/** Build-time marker written by the Dockerfile: "both" | "selkies" | "kasmvnc". */
export const DESKTOP_STACK_MARKER_PATH = "/etc/sdw-desktop-stack";

export const DESKTOP_CACHE_DIR = "/tmp/.desktop-cache";
export const DESKTOP_APPS_DIR = "/opt/desktop-apps";
export const DESKTOP_APPS_REGISTRY = `${DESKTOP_APPS_DIR}/.registry.json`;
/** Where installed desktop apps register their .desktop entries. */
export const DESKTOP_APPLICATIONS_DIR = "/usr/share/applications";

// Selkies stack: H.264/WebCodecs, served at /desktop/.
/** Selkies' own default (8082) collides with code-server, so we use 6080. */
export const SELKIES_STREAM_PORT = 6080;
export const SELKIES_DISPLAY = ":1";
export const SELKIES_HOME = `${WORKSPACE_MOUNT}/.desktop`;
/** Static Selkies web client, served by Caddy. */
export const SELKIES_WEB_ROOT = "/usr/share/selkies/web";

// KasmVNC stack: integrated X + VNC server, served at /vnc/.
export const VNC_STREAM_PORT = 6081;
export const VNC_DISPLAY = ":2";
export const VNC_HOME = `${WORKSPACE_MOUNT}/.desktop-vnc`;
/** KasmVNC serves its own client from its own package. */
export const VNC_WEB_ROOT = "/usr/share/kasmvnc/www";
