import { describe, expect, it } from "bun:test";
import { readFileSync, readdirSync, statSync } from "node:fs";
import path from "node:path";
import { ENV_GROUPS } from "@sdw/core/plugins";
import {
  CORE_ENV_GROUPS,
  INTERNAL_ENV,
  REQUIRED_ENV_VARS,
  flattenEnv,
  renderEnvExample,
} from "@sdw/core/workspace";

/**
 * The environment is declared once (core/workspace/env-schema.ts, plus each plugin's
 * variables). These tests keep that declaration, `.env.example`, the code that reads the
 * environment and docker-compose.yml from disagreeing:
 *   - the example file IS the generated one;
 *   - nothing is read that is not declared, and nothing is declared that nothing uses;
 *   - the example holds placeholders, never something that looks like a credential.
 */
const ROOT = path.resolve(import.meta.dir, "../../..");
const read = (rel: string) => readFileSync(path.join(ROOT, rel), "utf-8");
const SCHEMA_FILE = "packages/core/src/workspace/env-schema.ts";

const specs = flattenEnv(ENV_GROUPS);
const declared = new Set(specs.map((v) => v.name));

function walk(dir: string, accept: (file: string) => boolean): string[] {
  const out: string[] = [];
  for (const name of readdirSync(path.join(ROOT, dir))) {
    if (name === "node_modules" || name === "dist" || name === "target") continue;
    const rel = `${dir}/${name}`;
    const stat = statSync(path.join(ROOT, rel));
    if (stat.isDirectory()) out.push(...walk(rel, accept));
    else if (accept(rel)) out.push(rel);
  }
  return out;
}

/** Source of every package (not tests), minus the schema itself. */
function sources(): string[] {
  return readdirSync(path.join(ROOT, "packages"))
    .flatMap((pkg) => {
      try {
        return walk(`packages/${pkg}/src`, (f) => f.endsWith(".ts"));
      } catch {
        return [];
      }
    })
    .filter((f) => f !== SCHEMA_FILE);
}

/** Names the TypeScript reads from an environment object. */
function namesRead(): Set<string> {
  const patterns = [
    /value\(env, "([A-Z][A-Z0-9_]+)"\)/g,
    /\benv\.([A-Z][A-Z0-9_]+)/g,
    /process\.env\.([A-Z][A-Z0-9_]+)/g,
    /\benv\["([A-Z][A-Z0-9_]+)"\]/g,
  ];
  const found = new Set<string>();
  for (const file of sources()) {
    const code = read(file);
    for (const re of patterns) for (const m of code.matchAll(re)) found.add(m[1]);
  }
  return found;
}

describe("the schema", () => {
  it("declares each variable once", () => {
    const names = specs.map((v) => v.name);
    expect(names.filter((n, i) => names.indexOf(n) !== i)).toEqual([]);
  });

  it("every variable is upper snake case", () => {
    for (const v of specs) expect(v.name).toMatch(/^[A-Z][A-Z0-9_]*$/);
  });

  it("agrees with the start-up on which variables are required", () => {
    const required = flattenEnv(CORE_ENV_GROUPS)
      .filter((v) => v.required)
      .map((v) => v.name);
    expect([...REQUIRED_ENV_VARS] as string[]).toEqual(required);
  });

  it("a required variable is never commented out in the example", () => {
    for (const v of specs.filter((s) => s.required)) expect(v.optIn).toBeFalsy();
  });
});

describe(".env.example", () => {
  it("is exactly what the schema generates (run `make env-example` after editing it)", () => {
    expect(read(".env.example")).toBe(renderEnvExample(ENV_GROUPS));
  });

  it("holds placeholders, never anything that looks like a credential", () => {
    const text = read(".env.example");
    const suspicious: [string, RegExp][] = [
      ["a UUID", /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i],
      ["a GitHub token", /\bgh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}/],
      ["an AWS access key id", /\bAKIA[0-9A-Z]{12,}/],
      ["an API key", /\bsk-[A-Za-z0-9]{20,}/],
      ["credentials inside a URL", /\w+:\/\/[^\s/@:]+:[^\s/@]+@/],
    ];
    for (const [what, re] of suspicious) {
      expect({ what, found: re.test(text) }).toEqual({ what, found: false });
    }
  });

  it("leaves every secret empty", () => {
    for (const v of specs) {
      if (/(PASSWORD|SECRET|TOKEN|ACCESS_KEY_ID|GIT_REPO)$/.test(v.name)) {
        expect({ name: v.name, example: v.example }).toEqual({ name: v.name, example: "" });
      }
    }
  });

  it("uses example.com for hostnames and URLs", () => {
    for (const v of specs) {
      if (/^https?:\/\//.test(v.example)) expect(v.example).toContain("example.com");
      if (v.name.endsWith("HOSTNAME")) expect(v.example).toContain("example.com");
    }
  });
});

describe("the schema and the code agree", () => {
  it("every variable the code reads is declared (or internal)", () => {
    const undeclared = [...namesRead()].filter(
      (n) => !declared.has(n) && !INTERNAL_ENV.includes(n),
    );
    expect(undeclared).toEqual([]);
  });

  it("every ${VAR} in docker-compose.yml is declared", () => {
    const compose = read("docker-compose.yml");
    const used = [...compose.matchAll(/\$\{([A-Z][A-Z0-9_]+)/g)].map((m) => m[1]);
    expect(used.filter((n) => !declared.has(n))).toEqual([]);
  });

  it("no declared variable is dead: something in the repo uses each one", () => {
    // Search everything that can consume the environment, minus the schema that declares it.
    const corpus = [
      ...sources(),
      "docker-compose.yml",
      "bootstrap.sh",
      "destroy.sh",
      ...walk("scripts", () => true),
      ...walk("image", (f) => !f.includes("selkies-web")),
      ...walk("deploy", () => true),
    ]
      .map((f) => {
        try {
          return read(f);
        } catch {
          return "";
        }
      })
      .join("\n");
    const dead = specs
      .filter((v) => !v.external)
      .map((v) => v.name)
      .filter((name) => !new RegExp(`\\b${name}\\b`).test(corpus));
    // A documented variable that nothing reads is a lie in the example file.
    expect(dead).toEqual([]);
  });
});
