import { describe, expect, it } from "bun:test";
import fc from "fast-check";
import {
  REQUIRED_ENV_VARS,
  envValidationError,
  findMissingRequiredEnv,
} from "../src/lib/env-validate.ts";

/**
 * Property 18: Env Validation Completeness (R12.4)
 *
 * The check is pure so the invariant is verifiable without spawning a container.
 */
describe("Property 18: Env Validation Completeness", () => {
  const allRequired = [...REQUIRED_ENV_VARS];

  it("omitting any subset of required vars names at least one missing var", () => {
    fc.assert(
      fc.property(
        // a random subset of required vars to *include* (the rest are omitted)
        fc.subarray(allRequired),
        fc.dictionary(fc.string(), fc.string()),
        (present, extras) => {
          const env: Record<string, string | undefined> = { ...extras };
          for (const name of present) env[name] = "some-value";

          const missing = findMissingRequiredEnv(env, allRequired);
          const error = envValidationError(missing);

          const omittedCount = allRequired.length - present.length;
          if (omittedCount > 0) {
            expect(missing.length).toBe(omittedCount);
            expect(error).not.toBeNull();
            // The message must explicitly name at least one missing variable.
            expect(missing.some((v) => error?.includes(v))).toBe(true);
          } else {
            expect(missing).toEqual([]);
            expect(error).toBeNull();
          }
        },
      ),
    );
  });

  it("a present-but-empty var counts as missing", () => {
    fc.assert(
      fc.property(fc.constantFrom(...allRequired), (name) => {
        const env: Record<string, string | undefined> = {};
        for (const v of allRequired) env[v] = "x";
        env[name] = ""; // present but empty
        const missing = findMissingRequiredEnv(env, allRequired);
        expect(missing).toContain(name);
      }),
    );
  });

  it("a complete required set passes validation", () => {
    const env: Record<string, string | undefined> = {};
    for (const v of allRequired) env[v] = "value";
    expect(findMissingRequiredEnv(env, allRequired)).toEqual([]);
    expect(envValidationError([])).toBeNull();
  });
});
