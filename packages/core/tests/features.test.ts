import { describe, expect, it } from "bun:test";
import fc from "fast-check";
import { parseFeatureFlags } from "../src/workspace/features.ts";

describe("parseFeatureFlags", () => {
  it("is off by default", () => {
    expect(parseFeatureFlags({})).toEqual({ sync: false, remoteDestruct: false });
  });

  it("accepts 1 / true / yes / on in any case, ignoring surrounding spaces", () => {
    for (const v of ["1", "true", "TRUE", "Yes", " on ", "On"]) {
      expect(parseFeatureFlags({ SYNC_ENABLED: v }).sync).toBe(true);
      expect(parseFeatureFlags({ REMOTE_DESTRUCT_ENABLED: v }).remoteDestruct).toBe(true);
    }
  });

  it("treats an empty value and everything else as off", () => {
    fc.assert(
      fc.property(
        fc.string().filter((s) => !["1", "true", "yes", "on"].includes(s.trim().toLowerCase())),
        (v) => {
          expect(parseFeatureFlags({ SYNC_ENABLED: v, REMOTE_DESTRUCT_ENABLED: v })).toEqual({
            sync: false,
            remoteDestruct: false,
          });
        },
      ),
    );
    expect(parseFeatureFlags({ SYNC_ENABLED: "", REMOTE_DESTRUCT_ENABLED: undefined }).sync).toBe(
      false,
    );
  });

  it("the two flags are independent", () => {
    expect(parseFeatureFlags({ SYNC_ENABLED: "1" })).toEqual({ sync: true, remoteDestruct: false });
    expect(parseFeatureFlags({ REMOTE_DESTRUCT_ENABLED: "1" })).toEqual({
      sync: false,
      remoteDestruct: true,
    });
  });
});
