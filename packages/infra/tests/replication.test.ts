import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { $ } from "bun";
import {
  UNATTENDED_SSH,
  commitStaged,
  ensureCipherRepo,
  hasUnpushedCommits,
  originOf,
  pushToOrigin,
  stageChanges,
} from "../src/replication.ts";

/**
 * Real git against a local bare repository standing in for GitHub. Found by running the
 * container: a vault made by the start-up itself was never a git repository, so the
 * replication job crashed every 30 minutes and nothing was ever pushed; and a commit whose
 * push failed was never pushed again, because the next cycle looked like a quiet one.
 */
const GITIGNORE = "shared/\n";
const GITATTRIBUTES = "*.bin filter=lfs diff=lfs merge=lfs -text\n";
const WHO = { fallbackName: "Vault", fallbackEmail: "vault@local" };

let root: string;
let cipher: string;
let remote: string;

// No identity anywhere: the commit must not depend on one being configured.
const SAVED = { ...process.env };
beforeAll(() => {
  process.env.GIT_CONFIG_GLOBAL = path.join(tmpdir(), "sdw-replication-no-such-gitconfig");
  process.env.GIT_CONFIG_NOSYSTEM = "1";
  for (const key of [
    "GIT_AUTHOR_NAME",
    "GIT_AUTHOR_EMAIL",
    "GIT_COMMITTER_NAME",
    "GIT_COMMITTER_EMAIL",
  ]) {
    delete process.env[key];
  }
});
afterAll(() => {
  for (const key of ["GIT_CONFIG_GLOBAL", "GIT_CONFIG_NOSYSTEM"]) {
    if (SAVED[key] === undefined) delete process.env[key];
    else process.env[key] = SAVED[key];
  }
});

beforeEach(async () => {
  root = mkdtempSync(path.join(tmpdir(), "sdw-replication-"));
  cipher = path.join(root, "cipher");
  remote = path.join(root, "remote.git");
  mkdirSync(cipher);
  await $`git init -q --bare -b main ${remote}`.quiet();
  writeFileSync(path.join(cipher, "gocryptfs.conf"), "{}");
  writeFileSync(path.join(cipher, "AAAA"), "ciphertext one");
});
afterEach(() => rmSync(root, { recursive: true, force: true }));

const ensure = (url = remote) =>
  ensureCipherRepo({
    dir: cipher,
    remote: url,
    gitignore: GITIGNORE,
    gitattributes: GITATTRIBUTES,
  });
const commit = (message = "auto-sync") => commitStaged({ dir: cipher, message, ...WHO });
const push = () => pushToOrigin({ dir: cipher, ...WHO });
/** What one cron cycle does, minus the planning. */
const cycle = async (message = "auto-sync") => {
  const staged = await stageChanges(cipher);
  if (staged && !(await commit(message))) return false;
  return push();
};
const remoteLog = async () =>
  (await $`git --git-dir=${remote} log --format=%s main`.quiet().nothrow()).stdout
    .toString()
    .trim()
    .split("\n")
    .filter(Boolean);

describe("ensureCipherRepo", () => {
  it("turns a plain cipher directory into a repository whose origin is the remote", async () => {
    expect(await originOf(cipher)).toBeUndefined();
    expect(await ensure()).toBe("created");
    expect(await originOf(cipher)).toBe(remote);
    expect(readFileSync(path.join(cipher, ".gitignore"), "utf-8")).toBe(GITIGNORE);
    expect(readFileSync(path.join(cipher, ".gitattributes"), "utf-8")).toBe(GITATTRIBUTES);
  });

  it("is idempotent", async () => {
    await ensure();
    expect(await ensure()).toBe("unchanged");
    expect(await originOf(cipher)).toBe(remote);
  });

  it("follows a changed VAULT_GIT_REPO", async () => {
    await ensure();
    const other = path.join(root, "other.git");
    expect(await ensure(other)).toBe("remote-updated");
    expect(await originOf(cipher)).toBe(other);
  });

  it("adopts a repository made earlier, and keeps the files it already has", async () => {
    await $`git -C ${cipher} init -q -b main`.quiet();
    writeFileSync(path.join(cipher, ".gitignore"), "mine\n");
    expect(await ensure()).toBe("unchanged");
    expect(readFileSync(path.join(cipher, ".gitignore"), "utf-8")).toBe("mine\n");
    expect(await originOf(cipher)).toBe(remote);
  });

  it("makes ssh remotes work unattended: trust a new host once, never ask, never hang", async () => {
    await ensure();
    const configured = await $`git -C ${cipher} config --local --get core.sshCommand`.quiet();
    expect(configured.stdout.toString().trim()).toBe(UNATTENDED_SSH);
    expect(UNATTENDED_SSH).toContain("StrictHostKeyChecking=accept-new"); // not "no": a changed key is refused
    expect(UNATTENDED_SSH).toContain("BatchMode=yes");
  });

  it("also does so for a repository made earlier, and leaves a command the user chose alone", async () => {
    await $`git -C ${cipher} init -q -b main`.quiet();
    await ensure();
    const first = await $`git -C ${cipher} config --local --get core.sshCommand`.quiet();
    expect(first.stdout.toString().trim()).toBe(UNATTENDED_SSH);

    await $`git -C ${cipher} config --local core.sshCommand ${"ssh -i /my/key"}`.quiet();
    await ensure();
    const kept = await $`git -C ${cipher} config --local --get core.sshCommand`.quiet();
    expect(kept.stdout.toString().trim()).toBe("ssh -i /my/key");
  });

  it("throws, rather than pretending, when git cannot work in the directory", async () => {
    await expect(
      ensureCipherRepo({
        dir: path.join(root, "does-not-exist"),
        remote,
        gitignore: GITIGNORE,
        gitattributes: GITATTRIBUTES,
      }),
    ).rejects.toThrow();
  });
});

describe("originOf", () => {
  it("is undefined without a repository and without an origin", async () => {
    expect(await originOf(cipher)).toBeUndefined();
    await $`git -C ${cipher} init -q -b main`.quiet();
    expect(await originOf(cipher)).toBeUndefined();
  });
});

describe("a replication cycle", () => {
  it("replicates the first state, then only what changed", async () => {
    await ensure();
    expect(await stageChanges(cipher)).toBe(true);
    expect(await commit("first")).toBe(true);
    expect(await push()).toBe(true);
    expect(await remoteLog()).toEqual(["first"]);

    // Quiet cycle: nothing new, nothing to commit, nothing waiting to be pushed.
    expect(await stageChanges(cipher)).toBe(false);
    expect(await hasUnpushedCommits(cipher)).toBe(false);

    writeFileSync(path.join(cipher, "BBBB"), "ciphertext two");
    expect(await cycle("second")).toBe(true);
    expect(await remoteLog()).toEqual(["second", "first"]);
  });

  it("commits without any configured git identity", async () => {
    await ensure();
    await stageChanges(cipher);
    expect(await commit()).toBe(true);
    expect(await push()).toBe(true);
    const author = await $`git --git-dir=${remote} log -1 --format=%an,%ae main`.quiet();
    expect(author.stdout.toString().trim()).toBe("Vault,vault@local");
  });

  it("uses the identity git already has when there is one", async () => {
    await ensure();
    await $`git -C ${cipher} config user.name Ada`.quiet();
    await $`git -C ${cipher} config user.email ada@example.test`.quiet();
    await stageChanges(cipher);
    expect(await commit()).toBe(true);
    expect(await push()).toBe(true);
    const author = await $`git --git-dir=${remote} log -1 --format=%an,%ae main`.quiet();
    expect(author.stdout.toString().trim()).toBe("Ada,ada@example.test");
  });

  it("rebases and pushes again when another copy pushed first", async () => {
    await ensure();
    expect(await cycle("first")).toBe(true);

    // Another machine pushes a commit of its own.
    const other = path.join(root, "other-clone");
    await $`git clone -q ${remote} ${other}`.quiet();
    writeFileSync(path.join(other, "CCCC"), "from elsewhere");
    await $`git -C ${other} add -A`.quiet();
    await $`git -C ${other} -c user.name=Other -c user.email=o@example.test commit -q -m elsewhere`.quiet();
    await $`git -C ${other} push -q origin main`.quiet();

    writeFileSync(path.join(cipher, "BBBB"), "ciphertext two");
    expect(await cycle("second")).toBe(true);
    expect(await remoteLog()).toEqual(["second", "elsewhere", "first"]);
  });

  it("leaves no repository behind when nothing asked for one", async () => {
    expect(existsSync(path.join(cipher, ".git"))).toBe(false);
    expect(await stageChanges(cipher)).toBe(false);
    expect(await hasUnpushedCommits(cipher)).toBe(false);
  });
});

describe("a push that fails", () => {
  it("reports failure, does not throw, and the commit is still waiting afterwards", async () => {
    await ensure(path.join(root, "nowhere.git"));
    expect(await cycle("first")).toBe(false);
    // The commit happened; only the push did not. This is what the next cycle must notice.
    expect(await stageChanges(cipher)).toBe(false);
    expect(await hasUnpushedCommits(cipher)).toBe(true);
  });

  it("is pushed by a later cycle once the remote is back, with nothing else changed", async () => {
    await ensure();
    await $`rm -rf ${remote}`.quiet();
    expect(await cycle("while the remote is down")).toBe(false);
    expect(await cycle()).toBe(false); // still down: the retry fails too, it is not skipped

    await $`git init -q --bare -b main ${remote}`.quiet();
    expect(await stageChanges(cipher)).toBe(false); // looks like a quiet cycle ...
    expect(await hasUnpushedCommits(cipher)).toBe(true); // ... but a commit is waiting
    expect(await push()).toBe(true);
    expect(await remoteLog()).toEqual(["while the remote is down"]);
    expect(await hasUnpushedCommits(cipher)).toBe(false);
  });

  it("a never-pushed repository counts as having something to push", async () => {
    await ensure();
    await stageChanges(cipher);
    await commit();
    expect(await hasUnpushedCommits(cipher)).toBe(true);
    expect(await push()).toBe(true);
    expect(await hasUnpushedCommits(cipher)).toBe(false);
  });
});
