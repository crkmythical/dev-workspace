import { describe, expect, it } from "bun:test";
import {
  DESKTOP_STACK_MARKER_PATH,
  SELKIES_DISPLAY,
  SELKIES_HOME,
  SELKIES_STREAM_PORT,
  SELKIES_WEB_ROOT,
  VNC_DISPLAY,
  VNC_HOME,
  VNC_STREAM_PORT,
  VNC_WEB_ROOT,
} from "../src/desktop/layout.ts";
import { CLASH_CONTROLLER_PORT, CLASH_HTTP_PORT, CLASH_SOCKS_PORT } from "../src/egress/layout.ts";
import { CREDENTIALS_DIR, WORKSPACE_MOUNT } from "../src/shared/layout.ts";
import { SYNC_SERVICE_PORT } from "../src/sync/layout.ts";
import { CADDY_PORT, CODE_SERVER_PORT } from "../src/workspace/layout.ts";

/**
 * The container topology is a set of values that must not collide. They live in
 * per-context layout modules, so the cross-context invariants are pinned here.
 */

describe("in-container ports", () => {
  const ports = {
    CADDY_PORT,
    SYNC_SERVICE_PORT,
    CODE_SERVER_PORT,
    CLASH_HTTP_PORT,
    CLASH_SOCKS_PORT,
    CLASH_CONTROLLER_PORT,
    SELKIES_STREAM_PORT,
    VNC_STREAM_PORT,
  };

  it("are all distinct", () => {
    const values = Object.values(ports);
    expect(new Set(values).size).toBe(values.length);
  });

  it("keep their documented values", () => {
    expect(SELKIES_STREAM_PORT).toBe(6080);
    expect(VNC_STREAM_PORT).toBe(6081);
    expect(CADDY_PORT).toBe(8080);
    expect(SYNC_SERVICE_PORT).toBe(8081);
    expect(CODE_SERVER_PORT).toBe(8082);
  });
});

describe("dual-desktop layout", () => {
  it("the stacks use different X displays and different HOME dirs", () => {
    expect(SELKIES_DISPLAY).toBe(":1");
    expect(VNC_DISPLAY).toBe(":2");
    expect(VNC_DISPLAY).not.toBe(SELKIES_DISPLAY);
    expect(VNC_HOME).toBe("/workspace/.desktop-vnc");
    expect(SELKIES_HOME).toBe("/workspace/.desktop");
    expect(VNC_HOME).not.toBe(SELKIES_HOME);
  });

  it("each stack serves its client from its own package", () => {
    expect(SELKIES_WEB_ROOT).toBe("/usr/share/selkies/web");
    expect(VNC_WEB_ROOT).toBe("/usr/share/kasmvnc/www");
  });

  it("the stack marker path matches the one the Dockerfile writes", () => {
    expect(DESKTOP_STACK_MARKER_PATH).toBe("/etc/sdw-desktop-stack");
  });
});

describe("workspace layout is derived from one root", () => {
  it("keeps the vault mount and credentials where the image expects them", () => {
    expect(WORKSPACE_MOUNT).toBe("/workspace");
    expect(CREDENTIALS_DIR).toBe("/workspace/.credentials");
  });
});
