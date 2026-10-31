/**
 * Architecture ratchet for the TypeScript that still ships: the browser SPA and the
 * sync client it shares with it. The Rust migration retired packages/{server,cli,infra}
 * and the Rust-reimplemented domain logic in core (see docs/rust-migration.md), so only
 * two packages remain and the invariants that still matter are the browser ones:
 *
 *   R2  core depends on no other workspace package (it is a leaf)
 *   R3  spa depends only on core
 *   R4  core and spa import no node:/bun runtime (they run in the browser)
 *   R5  cross-package imports use @sdw/* specifiers, never relative paths
 *   R10 no hub imports: "@sdw/core" / "@sdw/core/constants" do not exist
 *
 * A new violation fails the build; so does fixing one without deleting its entry
 * from KNOWN_VIOLATIONS. Empty: every rule currently holds everywhere.
 */
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import path from "node:path";

const ROOT = path.resolve(import.meta.dir, "..");
const PACKAGES = ["core", "spa"] as const;
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

const KNOWN_VIOLATIONS = new Set<string>([]);

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

// ── rules ─────────────────────────────────────────────────────────────────────

function violationsOf(sources: Source[]): string[] {
  const found = new Set<string>();
  const add = (rule: string, file: string, detail: string) =>
    found.add(`${rule} ${file} -> ${detail}`);

  for (const e of collectEdges(sources)) {
    const reaches = e.target;
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
    if (e.spec === "@sdw/core" || e.spec === "@sdw/core/constants") {
      add("R10-no-hub-imports", e.file, e.spec);
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

  test("scans every package", () => {
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
