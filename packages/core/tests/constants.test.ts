import { describe, expect, it } from "bun:test";
import {
  CODE_SERVER_PORT,
  DESKTOP_DISPLAY,
  DESKTOP_STREAM_PORT,
  DESKTOP_WEB_ROOT,
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
