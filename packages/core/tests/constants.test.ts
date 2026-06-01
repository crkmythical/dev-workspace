import { describe, expect, it } from "bun:test";
import {
  CODE_SERVER_PORT,
  DESKTOP_DISPLAY,
  DESKTOP_STREAM_PORT,
  DESKTOP_WEB_ROOT,
  SELKIES_DISPLAY,
  SELKIES_STREAM_PORT,
  VNC_DISPLAY,
  VNC_HOME,
  VNC_STREAM_PORT,
  VNC_WEB_ROOT,
} from "../src/constants.ts";

describe("Desktop constants", () => {
  it("DESKTOP_STREAM_PORT is 6080", () => {
    expect(DESKTOP_STREAM_PORT).toBe(6080);
  });

  it("DESKTOP_STREAM_PORT does not collide with CODE_SERVER_PORT", () => {
    expect(DESKTOP_STREAM_PORT).not.toBe(CODE_SERVER_PORT);
  });

  it("DESKTOP_DISPLAY is :1", () => {
    expect(DESKTOP_DISPLAY).toBe(":1");
  });

  it("DESKTOP_WEB_ROOT is the expected path", () => {
    expect(DESKTOP_WEB_ROOT).toBe("/usr/share/selkies/web");
  });
});

describe("Dual-desktop constants", () => {
  it("selkies aliases match canonical desktop constants", () => {
    expect(SELKIES_STREAM_PORT).toBe(DESKTOP_STREAM_PORT);
    expect(SELKIES_DISPLAY).toBe(DESKTOP_DISPLAY);
  });

  it("VNC stream port is 6081 (distinct from selkies and code-server)", () => {
    expect(VNC_STREAM_PORT).toBe(6081);
    expect(VNC_STREAM_PORT).not.toBe(SELKIES_STREAM_PORT);
    expect(VNC_STREAM_PORT).not.toBe(CODE_SERVER_PORT);
  });

  it("VNC display is :2 (distinct from selkies :1)", () => {
    expect(VNC_DISPLAY).toBe(":2");
    expect(VNC_DISPLAY).not.toBe(SELKIES_DISPLAY);
  });

  it("VNC HOME is independent of selkies HOME", () => {
    expect(VNC_HOME).toBe("/workspace/.desktop-vnc");
  });

  it("VNC web root is the KasmVNC client path", () => {
    expect(VNC_WEB_ROOT).toBe("/usr/share/kasmvnc/www");
  });
});
