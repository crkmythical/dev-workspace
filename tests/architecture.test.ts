/**
 * Architecture fitness test: dependency rules between workspace packages.
 *
 * This is a RATCHET. Every existing violation is listed in KNOWN_VIOLATIONS:
 *   - a NEW violation fails the build;
 *   - FIXING a violation without deleting its entry also fails,
 * so the list can only shrink. Target layout: core = pure rules + use cases,
 * infra = adapters, cli / server / spa = driving adapters.
 */
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import path from "node:path";

const ROOT = path.resolve(import.meta.dir, "..");
const PACKAGES = ["core", "cli", "server", "spa"] as const;
type Pkg = (typeof PACKAGES)[number];

type Kind = "relative" | "alias" | "runtime" | "third-party";

interface Edge {
  file: string;
  pkg: Pkg;
  spec: string;
  kind: Kind;
  /** The OTHER workspace package this import reaches (undefined = none / same package). */
  target?: Pkg;
}

/** Existing violations, each tagged with the migration phase that removes it. */
const KNOWN_VIOLATIONS = new Set<string>([
  // Phase 1: the destroy use case moves into core (+ infra); server stops importing cli.
  "R1-server-and-cli-are-independent packages/server/src/routes/destruct.ts -> cli",

  // Phase 1: key derivation moves behind a port (node adapter in infra); core must stay
  // runtime-neutral so the browser SPA can import it.
  "R4-core-and-spa-are-runtime-neutral packages/core/src/crypto-utils.ts -> node:crypto",

  // Phase 2: server and spa switch from relative paths to @sdw/* package specifiers.
  "R5-cross-package-imports-use-@sdw packages/server/src/crypto.ts -> core",
  "R5-cross-package-imports-use-@sdw packages/server/src/index.ts -> core",
  "R5-cross-package-imports-use-@sdw packages/server/src/routes/destruct.ts -> cli", // Phase 1
  "R5-cross-package-imports-use-@sdw packages/server/src/routes/destruct.ts -> core",
  "R5-cross-package-imports-use-@sdw packages/server/src/routes/doctor.ts -> core",
  "R5-cross-package-imports-use-@sdw packages/server/src/routes/download.ts -> core",
  "R5-cross-package-imports-use-@sdw packages/server/src/routes/upload.ts -> core",
  "R5-cross-package-imports-use-@sdw packages/spa/src/main.ts -> core",
]);

// ── import extraction ───────────────────────────────────────────────────────

function extractImports(source: string): string[] {
  const code = source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|\s)\/\/.*$/gm, "$1");
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
    const abs = path.resolve(ROOT, path.dirname(file), spec);
    const [top, owner] = path.relative(ROOT, abs).split(path.sep);
    const target = top === "packages" ? (owner as Pkg) : undefined;
    const crosses = target !== undefined && target !== pkg && PACKAGES.includes(target);
    return { ...base, kind: "relative", target: crosses ? target : undefined };
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

function collectEdges(): Edge[] {
  const edges: Edge[] = [];
  for (const pkg of PACKAGES) {
    const srcDir = path.join(ROOT, "packages", pkg, "src");
    for (const rel of new Bun.Glob("**/*.ts").scanSync({ cwd: srcDir })) {
      const file = `packages/${pkg}/src/${rel}`;
      const source = readFileSync(path.join(ROOT, file), "utf-8");
      for (const spec of extractImports(source)) edges.push(classify(file, pkg, spec));
    }
  }
  return edges;
}

// ── rules ───────────────────────────────────────────────────────────────────

function violationsOf(edges: Edge[]): string[] {
  const found = new Set<string>();
  const add = (rule: string, e: Edge, detail: string) =>
    found.add(`${rule} ${e.file} -> ${detail}`);

  for (const e of edges) {
    const reaches = e.target;
    const serverCli =
      (e.pkg === "server" && reaches === "cli") || (e.pkg === "cli" && reaches === "server");
    if (serverCli && reaches) add("R1-server-and-cli-are-independent", e, reaches);
    if (e.pkg === "core" && reaches) add("R2-core-is-a-leaf", e, reaches);
    if (e.pkg === "spa" && reaches && reaches !== "core") {
      add("R3-spa-depends-only-on-core", e, reaches);
    }
    if ((e.pkg === "core" || e.pkg === "spa") && e.kind === "runtime") {
      add("R4-core-and-spa-are-runtime-neutral", e, e.spec);
    }
    if (e.kind === "relative" && reaches) add("R5-cross-package-imports-use-@sdw", e, reaches);
  }
  return [...found].sort();
}

// ── tests ───────────────────────────────────────────────────────────────────

describe("architecture: dependency rules (ratchet)", () => {
  const found = violationsOf(collectEdges());

  test("no NEW violations", () => {
    expect(found.filter((k) => !KNOWN_VIOLATIONS.has(k))).toEqual([]);
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
