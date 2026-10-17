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

/**
 * Existing violations. A new one fails the build; so does fixing one without
 * deleting its entry here. Empty since phase 1: keep it that way.
 */
const KNOWN_VIOLATIONS = new Set<string>([]);

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
    // Adapters implement core's ports; they never reach into a driving adapter.
    if (e.pkg === "infra" && reaches && reaches !== "core") {
      add("R6-infra-depends-only-on-core", e, reaches);
    }
    // Inside core the dependency points inward: domain <- ports <- usecases.
    if (e.pkg === "core" && e.kind === "relative" && e.resolved) {
      const importer = path.basename(e.file);
      const imported = path.basename(e.resolved);
      const inward =
        (importer === "domain.ts" && (imported === "ports.ts" || imported === "usecases.ts")) ||
        (importer === "ports.ts" && imported === "usecases.ts");
      if (inward) add("R7-core-layers-point-inward", e, imported);
    }
  }
  return [...found].sort();
}

// ── tests ───────────────────────────────────────────────────────────────────

describe("architecture: dependency rules (ratchet)", () => {
  const found = violationsOf(collectEdges());

  test("no NEW violations", () => {
    expect(found.filter((k) => !KNOWN_VIOLATIONS.has(k))).toEqual([]);
  });

  test("scans every package, including infra", () => {
    const scanned = new Set(collectEdges().map((e) => e.pkg));
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
