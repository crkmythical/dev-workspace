import { describe, expect, it } from "bun:test";
import { CLASH_SOCKS_ADDR, buildCrocReceive, buildCrocSend } from "../src/transfer/index.ts";

describe("croc send args", () => {
  it("routes through Clash by default; the code is never an argument (it goes in CROC_SECRET)", () => {
    const args = buildCrocSend({ relay: "r.example:9009", paths: ["a.txt"] });
    expect(args).toEqual([
      "--relay",
      "r.example:9009",
      "--socks5",
      CLASH_SOCKS_ADDR,
      "send",
      "a.txt",
    ]);
    expect(args).not.toContain("--code");
  });

  it("goes direct for a LAN relay", () => {
    expect(buildCrocSend({ relay: "10.0.0.5:9009", socks: null, paths: ["a", "b"] })).toEqual([
      "--relay",
      "10.0.0.5:9009",
      "send",
      "a",
      "b",
    ]);
  });
});

describe("croc receive args", () => {
  it("auto-accepts into a target directory, through Clash; no code on the command line", () => {
    expect(buildCrocReceive({ relay: "r.example:9009", out: "/workspace" })).toEqual([
      "--relay",
      "r.example:9009",
      "--socks5",
      CLASH_SOCKS_ADDR,
      "--yes",
      "--out",
      "/workspace",
    ]);
  });

  it("direct mode omits the proxy", () => {
    const args = buildCrocReceive({ relay: "10.0.0.5:9009", socks: null, out: "/workspace" });
    expect(args).not.toContain("--socks5");
    expect(args).toEqual(["--relay", "10.0.0.5:9009", "--yes", "--out", "/workspace"]);
  });
});
