import { describe, expect, it } from "bun:test";
import fc from "fast-check";
import {
  REQUIRED_ENV_VARS,
  envValidationError,
  findMissingRequiredEnv,
  loadConfig,
} from "../src/workspace/config.ts";

describe("loadConfig", () => {
  it("is total and applies defaults on an empty environment", () => {
    expect(loadConfig({})).toEqual({
      clashSubscriptionUrl: "",
      password: undefined,
      desktopResolution: "1920x1080",
      hostPort: "18080",
      git: { userName: undefined, userEmail: undefined, vaultRepo: undefined },
      backup: {
        RESTIC_REPOSITORY: undefined,
        RESTIC_PASSWORD: undefined,
        AWS_ACCESS_KEY_ID: undefined,
        AWS_SECRET_ACCESS_KEY: undefined,
      },
      features: { sync: false, remoteDestruct: false },
    });
  });

  it("treats an empty value like an unset one", () => {
    const cfg = loadConfig({ PASSWORD: "", DESKTOP_RESOLUTION: "", VAULT_GIT_REPO: "" });
    expect(cfg.password).toBeUndefined();
    expect(cfg.desktopResolution).toBe("1920x1080");
    expect(cfg.git.vaultRepo).toBeUndefined();
  });

  it("reads every documented variable", () => {
    const cfg = loadConfig({
      CLASH_SUBSCRIPTION_URL: "https://example.test/sub",
      PASSWORD: "s3cret",
      DESKTOP_RESOLUTION: "1280x720",
      GIT_USER_NAME: "Ada",
      GIT_USER_EMAIL: "ada@example.test",
      VAULT_GIT_REPO: "git@example.test:ada/vault.git",
      RESTIC_REPOSITORY: "s3:https://s3.example.test/bucket",
      RESTIC_PASSWORD: "r",
      AWS_ACCESS_KEY_ID: "id",
      AWS_SECRET_ACCESS_KEY: "key",
      SYNC_ENABLED: "1",
      REMOTE_DESTRUCT_ENABLED: "true",
    });
    expect(cfg.clashSubscriptionUrl).toBe("https://example.test/sub");
    expect(cfg.password).toBe("s3cret");
    expect(cfg.desktopResolution).toBe("1280x720");
    expect(cfg.git).toEqual({
      userName: "Ada",
      userEmail: "ada@example.test",
      vaultRepo: "git@example.test:ada/vault.git",
    });
    expect(cfg.backup.RESTIC_REPOSITORY).toBe("s3:https://s3.example.test/bucket");
    expect(cfg.features).toEqual({ sync: true, remoteDestruct: true });
  });

  it("takes the published web port from TUNNEL_HOST_PORT, and only if it is a real port", () => {
    expect(loadConfig({ TUNNEL_HOST_PORT: "18081" }).hostPort).toBe("18081");
    expect(loadConfig({ TUNNEL_HOST_PORT: " 8080 " }).hostPort).toBe("8080");
    for (const bad of ["", "0", "65536", "99999", "abc", "80;rm", "-1", "1e3", "123456"]) {
      expect({ bad, port: loadConfig({ TUNNEL_HOST_PORT: bad }).hostPort }).toEqual({
        bad,
        port: "18080",
      });
    }
  });

  it("never mutates or retains the environment it was given", () => {
    const env = { PASSWORD: "a" };
    const cfg = loadConfig(env);
    env.PASSWORD = "b";
    expect(cfg.password).toBe("a");
  });
});

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
