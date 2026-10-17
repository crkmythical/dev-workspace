import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { initRepository, probeRepository, resticEnv } from "../src/restic.ts";

/**
 * The adapter runs `restic`; here a stand-in script plays it, so every answer restic can give
 * is exercised without a repository. classifyRepoProbe() itself is tested in core, and against
 * the real restic 0.17.3 in the container (a missing repository is exit 10, a wrong password
 * exit 12, a path that is a file exit 1).
 */
let dir: string;
beforeEach(() => {
  dir = mkdtempSync(path.join(tmpdir(), "sdw-restic-"));
});
afterEach(() => rmSync(dir, { recursive: true, force: true }));

/** A `restic` that exits with `code`, printing `stderr`, and records the proxy it was given. */
function fakeRestic(code: number, stderr = "") {
  const script = path.join(dir, "restic");
  writeFileSync(
    script,
    `#!/bin/sh\necho "$HTTPS_PROXY $http_proxy $1" > "${dir}/seen"\necho "${stderr}" >&2\nexit ${code}\n`,
  );
  chmodSync(script, 0o755);
  return { PATH: `${dir}:/usr/bin:/bin` };
}

describe("probeRepository", () => {
  it("readable with this password", async () => {
    expect((await probeRepository(fakeRestic(0))).verdict).toBe("ready");
  });

  it("restic's explicit 'no repository here' is the only thing that counts as missing", async () => {
    expect(
      (await probeRepository(fakeRestic(10, "Fatal: repository does not exist"))).verdict,
    ).toBe("missing");
  });

  it("a wrong password is not a missing repository", async () => {
    expect(
      (await probeRepository(fakeRestic(12, "Fatal: wrong password or no key found"))).verdict,
    ).toBe("wrong-password");
  });

  it("any other failure is unreachable, never missing", async () => {
    const result = await probeRepository(
      fakeRestic(1, "Fatal: unable to open config file: network"),
    );
    expect(result.verdict).toBe("unreachable");
    expect(result.detail).toContain("network");
  });

  it("restic not being installed is unreachable, and does not throw", async () => {
    const empty = path.join(dir, "empty");
    // PATH with nothing in it.
    const result = await probeRepository({ PATH: empty });
    expect(result.verdict).toBe("unreachable");
  });

  it("goes through the same proxy as everything else, and only reads (snapshots)", async () => {
    await probeRepository(fakeRestic(0));
    expect(readFileSync(path.join(dir, "seen"), "utf-8").trim()).toBe(
      "http://127.0.0.1:7890 http://127.0.0.1:7890 snapshots",
    );
  });
});

describe("initRepository", () => {
  it("reports whether restic init worked", async () => {
    expect(await initRepository(fakeRestic(0))).toBe(true);
    expect(await initRepository(fakeRestic(1, "config file already exists"))).toBe(false);
  });

  it("restic not being installed is a failure, not an exception", async () => {
    expect(await initRepository({ PATH: path.join(dir, "empty") })).toBe(false);
  });
});

describe("resticEnv", () => {
  it("keeps the caller's variables and drops undefined ones", () => {
    const env = resticEnv({ RESTIC_PASSWORD: "x", GONE: undefined });
    expect(env.RESTIC_PASSWORD).toBe("x");
    expect("GONE" in env).toBe(false);
    expect(env.HTTPS_PROXY).toBe("http://127.0.0.1:7890");
  });
});
