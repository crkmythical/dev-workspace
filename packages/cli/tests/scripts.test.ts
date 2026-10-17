import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import {
  chmodSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

/**
 * The host scripts, tested for what can be tested without a Docker host, a Cloudflare
 * account or systemd: syntax, the settings reader, the history scrubber, and the
 * regressions found in them. Each case runs bash in a throwaway directory.
 */
const ROOT = path.resolve(import.meta.dir, "../../..");
const script = (rel: string) => path.join(ROOT, rel);

let dir: string;

interface Ran {
  code: number;
  out: string;
  err: string;
}

/**
 * Tools a host script must never reach from a test. Each is shadowed by a stub that logs
 * its name and fails, so a regression that lets a script run on (say, a default hostname
 * sneaking back in) cannot open a browser login, install a binary or touch Docker; the
 * test just sees the stub log.
 */
const STUBBED = [
  "cloudflared",
  "brew",
  "curl",
  "docker",
  "gh",
  "launchctl",
  "loginctl",
  "resolvectl",
  "sudo",
  "systemctl",
];

function stubDir(root: string): { bin: string; log: string } {
  const bin = path.join(root, "stub-bin");
  const log = path.join(root, "stub-calls.log");
  if (!existsSync(bin)) {
    mkdirSync(bin, { recursive: true });
    for (const name of STUBBED) {
      const file = path.join(bin, name);
      writeFileSync(file, `#!/bin/sh\necho "${name} $*" >> "${log}"\nexit 97\n`);
      chmodSync(file, 0o755);
    }
  }
  return { bin, log };
}

function bash(args: string[], opts: { cwd?: string; env?: Record<string, string> } = {}): Ran {
  const home = opts.cwd ?? dir;
  const stubs = stubDir(dir);
  const r = Bun.spawnSync(["bash", ...args], {
    cwd: opts.cwd,
    // A clean environment: nothing from the developer's shell or .env leaks in, and the
    // external tools above are replaced by stubs.
    env: {
      PATH: `${stubs.bin}:/usr/bin:/bin`,
      HOME: home,
      ...opts.env,
    },
  });
  return { code: r.exitCode, out: r.stdout.toString(), err: r.stderr.toString() };
}

/** Which stubbed tools were called (empty when no script reached for one). */
function stubCalls(): string[] {
  const log = path.join(dir, "stub-calls.log");
  return existsSync(log) ? readFileSync(log, "utf-8").trim().split("\n") : [];
}

beforeEach(() => {
  dir = mkdtempSync(path.join(tmpdir(), "sdw-scripts-"));
});
afterEach(() => rmSync(dir, { recursive: true, force: true }));

function shellScripts(): string[] {
  const out = ["bootstrap.sh", "destroy.sh", "parallels/provision.sh"];
  for (const d of ["scripts", "scripts/lib", "deploy/linux-systemd"]) {
    for (const f of readdirSync(script(d))) if (f.endsWith(".sh")) out.push(`${d}/${f}`);
  }
  return out.sort();
}

describe("every host script parses", () => {
  for (const rel of shellScripts()) {
    it(rel, () => {
      const r = bash(["-n", script(rel)]);
      expect({ file: rel, code: r.code, err: r.err }).toEqual({ file: rel, code: 0, err: "" });
    });
  }
});

describe("env_value (scripts/lib/env.sh)", () => {
  /** Run env_value NAME [default] with PROJECT_DIR = dir, a given .env and extra env. */
  function envValue(
    name: string,
    opts: { dotenv?: string; env?: Record<string, string>; fallback?: string } = {},
  ): string {
    if (opts.dotenv !== undefined) writeFileSync(path.join(dir, ".env"), opts.dotenv);
    const fallback = opts.fallback === undefined ? "" : ` '${opts.fallback}'`;
    const r = bash(
      ["-c", `source '${script("scripts/lib/env.sh")}'; env_value ${name}${fallback}`],
      {
        cwd: dir,
        env: { PROJECT_DIR: dir, ...opts.env },
      },
    );
    expect(r.code).toBe(0);
    return r.out;
  }

  it("the process environment wins over .env", () => {
    expect(envValue("X_NAME", { dotenv: "X_NAME=from-file\n", env: { X_NAME: "from-env" } })).toBe(
      "from-env",
    );
  });

  it("falls back to .env, then to the default", () => {
    expect(envValue("X_NAME", { dotenv: "X_NAME=from-file\n" })).toBe("from-file");
    expect(envValue("X_NAME", { dotenv: "OTHER=1\n", fallback: "dflt" })).toBe("dflt");
    expect(envValue("X_NAME", { fallback: "dflt" })).toBe("dflt");
  });

  it("reads a value that contains '=' or spaces whole (it never sources the file)", () => {
    expect(envValue("URL", { dotenv: "URL=https://x.test/p?a=1&b=2\n" })).toBe(
      "https://x.test/p?a=1&b=2",
    );
    expect(envValue("MSG", { dotenv: "MSG=hello big world\n" })).toBe("hello big world");
  });

  it("does not run shell syntax found in a value", () => {
    const marker = path.join(dir, "pwned");
    expect(envValue("V", { dotenv: `V=$(touch ${marker})\n` })).toBe(`$(touch ${marker})`);
    expect(existsSync(marker)).toBe(false);
  });

  it("strips one pair of quotes and a CRLF ending", () => {
    expect(envValue("A", { dotenv: 'A="quoted"\n' })).toBe("quoted");
    expect(envValue("A", { dotenv: "A='single'\n" })).toBe("single");
    expect(envValue("A", { dotenv: "A=plain\r\n" })).toBe("plain");
  });

  it("ignores comments, takes the last assignment, and treats an empty value as unset", () => {
    expect(envValue("A", { dotenv: "# A=commented\nA=real\n" })).toBe("real");
    expect(envValue("A", { dotenv: "A=first\nA=second\n" })).toBe("second");
    expect(envValue("A", { dotenv: "A=\n", fallback: "dflt" })).toBe("dflt");
  });

  it("matches the whole name, not a prefix", () => {
    expect(envValue("TUNNEL", { dotenv: "TUNNEL_NAME=x\n", fallback: "none" })).toBe("none");
  });
});

describe("scrub_history (scripts/lib/scrub.sh)", () => {
  const scrub = (file: string) =>
    bash(["-c", `source '${script("scripts/lib/scrub.sh")}'; scrub_history '${file}'`], {
      cwd: dir,
    });

  it("removes the lines that mention the workspace and keeps the rest", () => {
    const file = path.join(dir, "history");
    writeFileSync(
      file,
      [
        "ls -la",
        "docker exec -it dev-workspace unlock-vault",
        ": 1700000000:0;cloudflared tunnel run x",
        "git status",
        "gocryptfs -init /x",
        "echo clash",
        "cd project",
        "",
      ].join("\n"),
    );
    expect(scrub(file).code).toBe(0);
    expect(readFileSync(file, "utf-8")).toBe("ls -la\ngit status\ncd project\n");
  });

  it("leaves an empty file when every line matches, without failing", () => {
    const file = path.join(dir, "history");
    writeFileSync(file, "vault\nclash\n");
    expect(scrub(file).code).toBe(0);
    expect(readFileSync(file, "utf-8")).toBe("");
  });

  it("keeps the file's mode and leaves no backup or temp copy of the original", () => {
    const histDir = path.join(dir, "hist");
    const tmpDir = path.join(dir, "tmp");
    mkdirSync(histDir);
    mkdirSync(tmpDir);
    const file = path.join(histDir, "history");
    writeFileSync(file, "secret vault line\nkeep\n");
    chmodSync(file, 0o600);
    const r = bash(["-c", `source '${script("scripts/lib/scrub.sh")}'; scrub_history '${file}'`], {
      cwd: dir,
      env: { TMPDIR: tmpDir },
    });
    expect(r.code).toBe(0);
    expect(statSync(file).mode & 0o777).toBe(0o600);
    expect(readFileSync(file, "utf-8")).toBe("keep\n");
    expect(readdirSync(histDir)).toEqual(["history"]); // no .bak next to it
    expect(readdirSync(tmpDir)).toEqual([]); // the working copy is gone too
  });

  it("a missing file is not an error", () => {
    expect(scrub(path.join(dir, "absent")).code).toBe(0);
  });
});

describe("regressions in the host scripts", () => {
  it("the host watcher runs the destroy script that exists (it lives at the repo root)", () => {
    const watcher = readFileSync(script("scripts/host-watcher.sh"), "utf-8");
    expect(watcher).toContain('bash "${PROJECT_DIR}/destroy.sh"');
    expect(watcher).not.toContain('"${SCRIPT_DIR}/destroy.sh"');
    expect(existsSync(script("destroy.sh"))).toBe(true);
    expect(existsSync(script("scripts/destroy.sh"))).toBe(false);
  });

  it("setup-cloudflared refuses to guess a hostname", () => {
    // Run a copy in a project directory that has no .env, so nothing real is reachable.
    const proj = path.join(dir, "proj");
    mkdirSync(path.join(proj, "scripts", "lib"), { recursive: true });
    cpSync(
      script("scripts/setup-cloudflared.sh"),
      path.join(proj, "scripts", "setup-cloudflared.sh"),
    );
    cpSync(script("scripts/lib/env.sh"), path.join(proj, "scripts", "lib", "env.sh"));
    const r = bash([path.join(proj, "scripts", "setup-cloudflared.sh")], { cwd: proj });
    expect(r.code).toBe(2);
    expect(r.err).toContain("no hostname");
    expect(r.out).not.toContain("Cloudflare Tunnel Setup");
    expect(stubCalls()).toEqual([]); // it stopped before reaching for cloudflared, brew or curl
  });

  it("no host script carries a default hostname", () => {
    for (const rel of shellScripts()) {
      const text = readFileSync(script(rel), "utf-8");
      expect(text).not.toMatch(/TUNNEL_HOSTNAME[^\n]*:-[A-Za-z0-9]/);
    }
  });
});

describe("deploy/linux-systemd/install.sh --dry-run", () => {
  const install = (...args: string[]) =>
    bash([script("deploy/linux-systemd/install.sh"), ...args], { cwd: dir });

  it("prints a tunnel unit that runs the configured tunnel and restarts on failure", () => {
    const r = install("--dry-run", "--cloudflared");
    expect(r.code).toBe(0);
    expect(stubCalls()).toEqual([]);
    expect(r.out).toContain("tunnel run dev-workspace");
    expect(r.out).toContain("Restart=always");
    expect(r.out).toContain("WantedBy=default.target");
    expect(r.out).not.toContain("host-watcher.sh");
  });

  it("prints a watcher unit that runs from the project directory", () => {
    const r = install("--dry-run", "--host-watcher");
    expect(r.code).toBe(0);
    expect(r.out).toContain(`WorkingDirectory=${ROOT}`);
    expect(r.out).toContain(`ExecStart=/bin/bash ${ROOT}/scripts/host-watcher.sh`);
    expect(existsSync(script("scripts/host-watcher.sh"))).toBe(true);
  });

  it("takes the tunnel name from the environment", () => {
    const r = bash([script("deploy/linux-systemd/install.sh"), "--dry-run", "--cloudflared"], {
      cwd: dir,
      env: { TUNNEL_NAME: "my-tunnel" },
    });
    expect(r.out).toContain("tunnel run my-tunnel");
  });

  it("installs nothing in dry-run mode", () => {
    const r = bash([script("deploy/linux-systemd/install.sh"), "--dry-run", "--cloudflared"], {
      cwd: dir,
      env: { XDG_CONFIG_HOME: path.join(dir, "cfg") },
    });
    expect(r.code).toBe(0);
    expect(existsSync(path.join(dir, "cfg"))).toBe(false);
  });

  it("asks for something to do, and rejects an unknown option", () => {
    expect(install().code).toBe(2);
    expect(install("--bogus").code).toBe(2);
  });
});
