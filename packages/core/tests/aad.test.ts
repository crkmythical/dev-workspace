import { describe, it, expect } from "bun:test";
import { buildAad, buildChunkAad, buildFinalizeAad } from "../src/aad.ts";

describe("buildAad", () => {
  it("joins op and timestamp", () => {
    const aad = buildAad("handshake", 1700000000000);
    expect(new TextDecoder().decode(aad)).toBe("handshake|1700000000000");
  });

  it("includes extra fields", () => {
    const aad = buildAad("put-chunk", 123, "file-1", "0");
    expect(new TextDecoder().decode(aad)).toBe("put-chunk|123|file-1|0");
  });
});

describe("buildChunkAad", () => {
  it("formats correctly", () => {
    const aad = buildChunkAad(999, "fid", 3);
    expect(new TextDecoder().decode(aad)).toBe("put-chunk|999|fid|3");
  });
});

describe("buildFinalizeAad", () => {
  it("formats correctly", () => {
    const aad = buildFinalizeAad(888, "fid", 10);
    expect(new TextDecoder().decode(aad)).toBe("put-finalize|888|fid|10");
  });
});
