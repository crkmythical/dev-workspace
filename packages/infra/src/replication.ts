/**
 * Replication of the vault's ciphertext to a git remote: the part that runs git.
 *
 * The repository lives in the cipher directory, outside the mount, and holds only
 * ciphertext and gocryptfs.conf. The container's start-up makes sure it exists and points
 * at VAULT_GIT_REPO (ensureCipherRepo); the cron job only ever commits and pushes
 * (stageChanges, commitStaged, pushToOrigin), because cron does not pass the container's
 * environment on.
 *
 * Nothing here prints: the remote URL may carry an access token.
 */
import { existsSync } from "node:fs";
import { $ } from "bun";

/** The URL of `origin`, or undefined when there is no repository or it has no such remote. */
export async function originOf(dir: string): Promise<string | undefined> {
  if (!existsSync(`${dir}/.git`)) return undefined;
  const r = await $`git -C ${dir} remote get-url origin`.quiet().nothrow();
  if (r.exitCode !== 0) return undefined;
  return r.stdout.toString().trim() || undefined;
}

/** What git runs for ssh:// and git@host: remotes when nobody is there to answer a question. */
export const UNATTENDED_SSH = "ssh -o StrictHostKeyChecking=accept-new -o BatchMode=yes";

export type EnsureResult = "created" | "remote-updated" | "unchanged";

/**
 * Make `dir` a git repository whose `origin` is `remote`. Idempotent. A vault made by the
 * container's own start-up used to have no repository at all, so replication could not work
 * unless the interactive init-vault had been run first. Files that exist are not rewritten.
 */
export async function ensureCipherRepo(opts: {
  dir: string;
  remote: string;
  gitignore: string;
  gitattributes: string;
}): Promise<EnsureResult> {
  const { dir, remote } = opts;
  let result: EnsureResult = "unchanged";

  if (!existsSync(`${dir}/.git`)) {
    const init = await $`git -C ${dir} init -q -b main`.quiet().nothrow();
    if (init.exitCode !== 0) throw new Error("git init failed in the cipher directory");
    result = "created";
  }
  if (!existsSync(`${dir}/.gitignore`)) await Bun.write(`${dir}/.gitignore`, opts.gitignore);
  if (!existsSync(`${dir}/.gitattributes`)) {
    await Bun.write(`${dir}/.gitattributes`, opts.gitattributes);
  }

  // The job that pushes runs from cron: nobody can answer ssh's "trust this host?" question,
  // and without an answer it refuses ("Host key verification failed", seen in a test with a real
  // sshd; the job's only message was "push failed"). The first sight of a server is trusted and
  // pinned in ~/.ssh/known_hosts (the vault's credentials); a key that changes later is still
  // refused. BatchMode keeps a key that needs a passphrase from hanging the job. A command the
  // user set for this repository is theirs and stays.
  const sshCommand = await $`git -C ${dir} config --local --get core.sshCommand`.quiet().nothrow();
  if (sshCommand.exitCode !== 0) {
    const set = await $`git -C ${dir} config --local core.sshCommand ${UNATTENDED_SSH}`
      .quiet()
      .nothrow();
    if (set.exitCode !== 0) throw new Error("could not set core.sshCommand");
  }

  const current = await originOf(dir);
  if (current === undefined) {
    const add = await $`git -C ${dir} remote add origin ${remote}`.quiet().nothrow();
    if (add.exitCode !== 0) throw new Error("could not add the origin remote");
  } else if (current !== remote) {
    const set = await $`git -C ${dir} remote set-url origin ${remote}`.quiet().nothrow();
    if (set.exitCode !== 0) throw new Error("could not update the origin remote");
    if (result === "unchanged") result = "remote-updated";
  }
  return result;
}

/** Stage everything. True when there is something to commit. */
export async function stageChanges(dir: string): Promise<boolean> {
  await $`git -C ${dir} add -A`.quiet().nothrow();
  // `diff --cached --quiet` exits 1 when something is staged.
  const diff = await $`git -C ${dir} diff --cached --quiet`.quiet().nothrow();
  return diff.exitCode === 1;
}

/**
 * Commits that are not on origin/main yet: all of them when it has never been pushed, the
 * ones after origin/main otherwise. A push that failed leaves its commit here.
 */
export async function hasUnpushedCommits(dir: string): Promise<boolean> {
  const head = await $`git -C ${dir} rev-parse --verify -q HEAD`.quiet().nothrow();
  if (head.exitCode !== 0) return false; // nothing committed yet
  const remote = await $`git -C ${dir} rev-parse --verify -q refs/remotes/origin/main`
    .quiet()
    .nothrow();
  if (remote.exitCode !== 0) return true;
  const ahead = await $`git -C ${dir} rev-list --count refs/remotes/origin/main..HEAD`
    .quiet()
    .nothrow();
  return Number(ahead.stdout.toString().trim()) > 0;
}

interface Identity {
  fallbackName: string;
  fallbackEmail: string;
}

/**
 * The identity git already has (boot sets it from GIT_USER_NAME / GIT_USER_EMAIL), else the
 * fallback, so a container without one does not fail with "who are you".
 */
async function identityArgs(dir: string, id: Identity): Promise<string[]> {
  const configured = async (key: string, fallback: string) => {
    const r = await $`git -C ${dir} config ${key}`.quiet().nothrow();
    return (r.exitCode === 0 && r.stdout.toString().trim()) || fallback;
  };
  return [
    "-c",
    `user.name=${await configured("user.name", id.fallbackName)}`,
    "-c",
    `user.email=${await configured("user.email", id.fallbackEmail)}`,
  ];
}

/** Commit what is staged. */
export async function commitStaged(
  opts: { dir: string; message: string } & Identity,
): Promise<boolean> {
  const who = await identityArgs(opts.dir, opts);
  const commit = await $`git -C ${opts.dir} ${who} commit -q -m ${opts.message}`.quiet().nothrow();
  return commit.exitCode === 0;
}

/**
 * Push to origin/main. If it is refused (another copy pushed first), rebase onto the remote
 * once and push again.
 */
export async function pushToOrigin(opts: { dir: string } & Identity): Promise<boolean> {
  const { dir } = opts;
  const push = () => $`git -C ${dir} push -q -u origin main`.quiet().nothrow();
  if ((await push()).exitCode === 0) return true;

  const who = await identityArgs(dir, opts);
  const rebase = await $`git -C ${dir} ${who} pull --rebase -q origin main`.quiet().nothrow();
  if (rebase.exitCode !== 0) return false;
  return (await push()).exitCode === 0;
}
