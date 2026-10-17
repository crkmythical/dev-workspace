/**
 * Architecture fitness test: dependency and layout rules for the workspace.
 *
 * This is a RATCHET. Every existing violation is listed in KNOWN_VIOLATIONS:
 *   - a NEW violation fails the build;
 *   - FIXING a violation without deleting its entry also fails,
 * so the list can only shrink. Target layout: core = pure rules + use cases,
 * infra = adapters, cli / server / spa = driving adapters.
 *
 *   R1  server and cli never depend on each other
 *   R2  core depends on no other workspace package
 *   R3  spa depends only on core
 *   R4  core and spa import no node:/bun runtime
 *   R5  cross-package imports use @sdw/* specifiers, never relative paths
 *   R6  infra depends only on core
 *   R7  inside core: domain <- ports <- usecases
 *   R8  process.env is read only at entry points (cli/src/*.ts, server/src/index.ts)
 *   R9  absolute path literals live only in layout.ts modules
 *   R10 no hub imports: "@sdw/core" and "@sdw/core/constants" no longer exist
 */
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import path from "node:path";

const ROOT = path.resolve(import.meta.dir, "..");
const PACKAGES = ["core", "infra", "cli", "server", "spa"] as const;
type Pkg = (typeof PACKAGES)[number];

type Kind = "relative" | "alias" | "runtime" | "third-party";

interface Edge {
  file: string;
  pkg: Pkg;
  spec: string;
  kind: Kind;
  /** The OTHER workspace package this import reaches (undefined = none / same package). */
  target?: Pkg;
  /** For relative imports: the resolved path, relative to the repo root. */
  resolved?: string;
}

interface Source {
  file: string;
  pkg: Pkg;
  /** Source text with comments removed. */
  code: string;
}

/**
 * Existing violations. A new one fails the build; so does fixing one without
 * deleting its entry here. The pentest rootfs paths disappear when the pentest
 * plugin gets its own layout module.
 */
const KNOWN_VIOLATIONS = new Set<string>([
  // Phase 5: these are paths INSIDE the pentest rootfs (and its bashrc template). They move
  // into the pentest plugin's own layout module when pentest becomes a self-contained plugin.
  "R9-absolute-paths-live-in-layout-modules packages/cli/src/lib/pentest-config.ts -> /usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/opt/tools/bin",
  "R9-absolute-paths-live-in-layout-modules packages/cli/src/pentest-bootstrap.ts -> /etc/apt/apt.conf.d/90proxy",
  "R9-absolute-paths-live-in-layout-modules packages/cli/src/pentest-bootstrap.ts -> /etc/hostname",
  "R9-absolute-paths-live-in-layout-modules packages/cli/src/pentest-bootstrap.ts -> /etc/proxychains4.conf",
  "R9-absolute-paths-live-in-layout-modules packages/cli/src/pentest-bootstrap.ts -> /etc/resolv.conf",
  "R9-absolute-paths-live-in-layout-modules packages/cli/src/pentest-bootstrap.ts -> /etc/tor/torrc",
  "R9-absolute-paths-live-in-layout-modules packages/cli/src/pentest-bootstrap.ts -> /opt/tools/install-kali-tools.sh",
  "R9-absolute-paths-live-in-layout-modules packages/cli/src/pentest-bootstrap.ts -> /root/.bashrc",
  "R9-absolute-paths-live-in-layout-modules packages/cli/src/pentest-enter.ts -> /tmp/.pentest-enter.sh",
]);

// ── source loading ──────────────────────────────────────────────────────────

function stripComments(source: string): string {
  return source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|\s)\/\/.*$/gm, "$1");
}

function loadSources(): Source[] {
  const out: Source[] = [];
  for (const pkg of PACKAGES) {
    const srcDir = path.join(ROOT, "packages", pkg, "src");
    for (const rel of new Bun.Glob("**/*.ts").scanSync({ cwd: srcDir })) {
      const file = `packages/${pkg}/src/${rel}`;
      out.push({ file, pkg, code: stripComments(readFileSync(path.join(ROOT, file), "utf-8")) });
    }
  }
  return out;
}

// ── import extraction ───────────────────────────────────────────────────────

function extractImports(source: string): string[] {
  const code = stripComments(source);
  const patterns = [
    /\b(?:import|export)\s+(?:type\s+)?[^;"'`]*?\bfrom\s*["']([^"']+)["']/g,
    /\bimport\s*["']([^"']+)["']/g,
    /\bimport\(\s*["']([^"']+)["']\s*\)/g,
  ];
  return patterns.flatMap((re) => [...code.matchAll(re)].map((m) => m[1]));
}

function classify(file: string, pkg: Pkg, spec: string): Edge {
  const base = { file, pkg, spec };
  if (spec.startsWith(".")) {
    const resolved = path.relative(ROOT, path.resolve(ROOT, path.dirname(file), spec));
    const [top, owner] = resolved.split(path.sep);
    const target = top === "packages" ? (owner as Pkg) : undefined;
    const crosses = target !== undefined && target !== pkg && PACKAGES.includes(target);
    return { ...base, kind: "relative", target: crosses ? target : undefined, resolved };
  }
  const alias = spec.match(/^@sdw\/([^/]+)/);
  if (alias) {
    const target = alias[1] as Pkg;
    return { ...base, kind: "alias", target: target !== pkg ? target : undefined };
  }
  if (spec.startsWith("node:") || spec === "bun" || spec.startsWith("bun:")) {
    return { ...base, kind: "runtime" };
  }
  return { ...base, kind: "third-party" };
}

function collectEdges(sources: Source[]): Edge[] {
  return sources.flatMap((s) =>
    extractImports(s.code).map((spec) => classify(s.file, s.pkg, spec)),
  );
}

// ── text rules ──────────────────────────────────────────────────────────────

/** A quoted string whose content STARTS with an absolute system path. */
const ABSOLUTE_PATH_LITERAL =
  /(["'`])(\/(?:workspace|vault|pentest|var\/run|etc|opt|root|usr|tmp)\b[^"'`\n]*)\1/g;

function findAbsolutePathLiterals(code: string): string[] {
  return [...code.matchAll(ABSOLUTE_PATH_LITERAL)].map((m) => m[2]);
}

function readsProcessEnv(code: string): boolean {
  return /\bprocess\.env\b/.test(code);
}

const isEntryPoint = (file: string) =>
  /^packages\/cli\/src\/[^/]+\.ts$/.test(file) || file === "packages/server/src/index.ts";

// ── rules ───────────────────────────────────────────────────────────────────

function violationsOf(sources: Source[]): string[] {
  const found = new Set<string>();
  const add = (rule: string, file: string, detail: string) =>
    found.add(`${rule} ${file} -> ${detail}`);

  for (const e of collectEdges(sources)) {
    const reaches = e.target;
    const serverCli =
      (e.pkg === "server" && reaches === "cli") || (e.pkg === "cli" && reaches === "server");
    if (serverCli && reaches) add("R1-server-and-cli-are-independent", e.file, reaches);
    if (e.pkg === "core" && reaches) add("R2-core-is-a-leaf", e.file, reaches);
    if (e.pkg === "spa" && reaches && reaches !== "core") {
      add("R3-spa-depends-only-on-core", e.file, reaches);
    }
    if ((e.pkg === "core" || e.pkg === "spa") && e.kind === "runtime") {
      add("R4-core-and-spa-are-runtime-neutral", e.file, e.spec);
    }
    if (e.kind === "relative" && reaches) {
      add("R5-cross-package-imports-use-@sdw", e.file, reaches);
    }
    // Adapters implement core's ports; they never reach into a driving adapter.
    if (e.pkg === "infra" && reaches && reaches !== "core") {
      add("R6-infra-depends-only-on-core", e.file, reaches);
    }
    // Inside core the dependency points inward: domain <- ports <- usecases.
    if (e.pkg === "core" && e.kind === "relative" && e.resolved) {
      const importer = path.basename(e.file);
      const imported = path.basename(e.resolved);
      const inward =
        (importer === "domain.ts" && (imported === "ports.ts" || imported === "usecases.ts")) ||
        (importer === "ports.ts" && imported === "usecases.ts");
      if (inward) add("R7-core-layers-point-inward", e.file, imported);
    }
    if (e.spec === "@sdw/core" || e.spec === "@sdw/core/constants") {
      add("R10-no-hub-imports", e.file, e.spec);
    }
  }

  for (const s of sources) {
    if (readsProcessEnv(s.code) && !isEntryPoint(s.file)) {
      add("R8-process-env-only-at-entrypoints", s.file, "process.env");
    }
    if (path.basename(s.file) !== "layout.ts") {
      for (const literal of new Set(findAbsolutePathLiterals(s.code))) {
        add("R9-absolute-paths-live-in-layout-modules", s.file, literal);
      }
    }
  }
  return [...found].sort();
}

// ── tests ───────────────────────────────────────────────────────────────────

describe("architecture: dependency and layout rules (ratchet)", () => {
  const sources = loadSources();
  const found = violationsOf(sources);

  test("no NEW violations", () => {
    expect(found.filter((k) => !KNOWN_VIOLATIONS.has(k))).toEqual([]);
  });

  test("scans every package, including infra", () => {
    const scanned = new Set(sources.map((s) => s.pkg));
    for (const pkg of PACKAGES) expect(scanned.has(pkg)).toBe(true);
  });

  test("KNOWN_VIOLATIONS has no stale entries (the list may only shrink)", () => {
    const foundSet = new Set(found);
    expect([...KNOWN_VIOLATIONS].filter((k) => !foundSet.has(k))).toEqual([]);
  });
});

describe("architecture: import extractor", () => {
  test("handles single-line, multi-line, type, re-export, side-effect and dynamic imports", () => {
    const src = `
      import { a } from "./a.ts";
      import type { B } from "@sdw/core/types";
      import {
        c,
        d,
      } from "node:fs";
      export * from "./e.ts";
      export { f } from "../f.ts";
      import "./side-effect.ts";
      const g = await import("./g.ts");
    `;
    expect(extractImports(src).sort()).toEqual(
      [
        "./a.ts",
        "@sdw/core/types",
        "node:fs",
        "./e.ts",
        "../f.ts",
        "./side-effect.ts",
        "./g.ts",
      ].sort(),
    );
  });

  test("ignores imports that only appear in comments", () => {
    const src = `
      // import { x } from "./line-comment.ts";
      /* import { y } from "./block-comment.ts"; */
      import { real } from "./real.ts";
    `;
    expect(extractImports(src)).toEqual(["./real.ts"]);
  });

  test("keeps URLs inside strings intact (does not treat // as a comment)", () => {
    const src = `
      const url = "http://127.0.0.1:7890";
      import { z } from "./z.ts";
    `;
    expect(extractImports(src)).toEqual(["./z.ts"]);
  });
});

describe("architecture: text rules", () => {
  test("finds a quoted absolute path, in any quote style", () => {
    const code = `const a = "/workspace/x"; const b = '/etc/clash'; const c = \`/var/run/y\`;`;
    expect(findAbsolutePathLiterals(code)).toEqual(["/workspace/x", "/etc/clash", "/var/run/y"]);
  });

  test("ignores paths built from a constant, and non-system roots", () => {
    const code = "const a = `${DIR}/x`; const b = \"/sync/api/upload\"; const c = '/not/a/root';";
    expect(findAbsolutePathLiterals(code)).toEqual([]);
  });

  test("a path inside prose or a comment does not count", () => {
    expect(findAbsolutePathLiterals(stripComments("// see /workspace/x\nconst a = 1;"))).toEqual(
      [],
    );
  });

  test("detects process.env access but not the word in a comment", () => {
    expect(readsProcessEnv("const x = process.env.HOME;")).toBe(true);
    expect(readsProcessEnv(stripComments("// call loadConfig(process.env) once"))).toBe(false);
  });

  test("entry points are the cli's top-level files and the server index only", () => {
    expect(isEntryPoint("packages/cli/src/doctor.ts")).toBe(true);
    expect(isEntryPoint("packages/server/src/index.ts")).toBe(true);
    expect(isEntryPoint("packages/cli/src/lib/passphrase.ts")).toBe(false);
    expect(isEntryPoint("packages/server/src/routes/upload.ts")).toBe(false);
    expect(isEntryPoint("packages/infra/src/destroy.ts")).toBe(false);
  });
});
