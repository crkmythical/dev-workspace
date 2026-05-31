import { describe, expect, it } from "bun:test";
import fc from "fast-check";
import { checkRelativePath, isSafeRelativePath } from "../src/path-safety.ts";

/**
 * Property tests for path-safety (T1 — upload/download traversal hardening).
 *
 * Invariant: a path accepted by checkRelativePath, when joined under a base
 * dir, can never escape that base. Encoded as: the normalized result has no
 * "..", no leading "/", no backslash, no NUL — and any input containing a
 * traversal segment is rejected.
 */

describe("checkRelativePath — rejects unsafe inputs", () => {
  it("rejects empty", () => {
    expect(checkRelativePath("").ok).toBe(false);
  });

  it("rejects absolute paths", () => {
    expect(checkRelativePath("/etc/passwd")).toEqual({ ok: false, reason: "absolute" });
  });

  it("rejects NUL bytes", () => {
    expect(checkRelativePath("a\0b")).toEqual({ ok: false, reason: "nul-byte" });
  });

  it("rejects backslashes", () => {
    expect(checkRelativePath("a\\b")).toEqual({ ok: false, reason: "backslash" });
  });

  it("rejects any path with a .. segment", () => {
    for (const p of ["..", "../x", "a/../../etc", "a/b/../../../etc/passwd", "foo/..", "./../x"]) {
      expect({ p, ok: checkRelativePath(p).ok }).toEqual({ p, ok: false });
    }
  });

  it("collapses ./ and accepts plain relative paths", () => {
    expect(checkRelativePath("a/./b")).toEqual({ ok: true, normalized: "a/b" });
    expect(checkRelativePath("docs/readme.md")).toEqual({ ok: true, normalized: "docs/readme.md" });
    expect(checkRelativePath("file.txt")).toEqual({ ok: true, normalized: "file.txt" });
  });
});

describe("Property: accepted paths never escape the base", () => {
  it("a normalized accepted path has no traversal, is never absolute", () => {
    fc.assert(
      fc.property(fc.string({ maxLength: 80 }), (s) => {
        const r = checkRelativePath(s);
        if (r.ok) {
          expect(r.normalized.startsWith("/")).toBe(false);
          expect(r.normalized.includes("\\")).toBe(false);
          expect(r.normalized.includes("\0")).toBe(false);
          // no ".." as a whole segment
          expect(r.normalized.split("/").includes("..")).toBe(false);
          expect(r.normalized.length).toBeGreaterThan(0);
        }
      }),
    );
  });

  it("any input containing a .. path segment is always rejected", () => {
    fc.assert(
      fc.property(
        fc.array(fc.constantFrom("a", "b", "..", ".", "x"), { minLength: 1, maxLength: 8 }),
        (segs) => {
          const input = segs.join("/");
          if (segs.includes("..")) {
            expect(isSafeRelativePath(input)).toBe(false);
          }
        },
      ),
    );
  });

  it("simulated join: resolved path stays under base for all accepted inputs", () => {
    const base = "/workspace/shared";
    fc.assert(
      fc.property(fc.string({ maxLength: 60 }), (s) => {
        const r = checkRelativePath(s);
        if (r.ok) {
          // POSIX join + normalize must remain prefixed by base.
          const joined = `${base}/${r.normalized}`;
          const parts = joined.split("/").filter((p) => p !== "" && p !== ".");
          expect(parts.includes("..")).toBe(false);
          expect(joined.startsWith(`${base}/`)).toBe(true);
        }
      }),
    );
  });
});
