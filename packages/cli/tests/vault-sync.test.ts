import { describe, it, expect } from "bun:test";
import fc from "fast-check";
import { planSync } from "../src/lib/vault-lifecycle.ts";
import { parseGitignore, isIgnored } from "../src/lib/gitignore-match.ts";

/**
 * Property tests for vault-sync (task 4.3).
 *
 *   Property 5:  Vault State Invariants on Locked Boundary (R5.8, 8.6, 9.6, 13.5)
 *   Property 15: Vault-Sync Exclusion Correctness          (R9.9, 25.1)
 *   Property 16: Vault-Sync No-Op on Quiet Cycle           (R9.3, 9.4)
 *
 * Property 5/16 sit on the pure sync planner. Property 15 validates the
 * exclusion matcher against the vault's baked .gitignore (init-vault.ts). The
 * bats harness (tests/property/vault-sync.bats) cross-checks the same paths
 * against git's own `check-ignore` to confirm semantic agreement.
 */

// The exact .gitignore body written by init-vault.ts.
const VAULT_GITIGNORE = `node_modules/
target/
build/
dist/
.cache/
**/*.log
.idea/
.vscode/
__pycache__/
.DS_Store
*.tmp
*.swp
shared/
`;

const rules = parseGitignore(VAULT_GITIGNORE);

describe("Property 5: Vault State Invariants on Locked Boundary", () => {
  it("never commits while the vault is locked (not mounted)", () => {
    fc.assert(
      fc.property(fc.boolean(), (hasStagedChanges) => {
        const plan = planSync({ mounted: false, hasStagedChanges });
        expect(plan.action).toBe("skip");
        if (plan.action === "skip") {
          expect(plan.reason).toBe("not-mounted");
          expect(plan.exitCode).toBe(0);
        }
      }),
    );
  });
});

describe("Property 16: Vault-Sync No-Op on Quiet Cycle", () => {
  it("mounted + no staged changes ⇒ skip, no commit", () => {
    const plan = planSync({ mounted: true, hasStagedChanges: false });
    expect(plan.action).toBe("skip");
    if (plan.action === "skip") expect(plan.reason).toBe("no-changes");
  });

  it("mounted + staged changes ⇒ commit-and-push", () => {
    expect(planSync({ mounted: true, hasStagedChanges: true }).action).toBe("commit-and-push");
  });

  it("only the (mounted ∧ changed) cell commits; the other three skip", () => {
    fc.assert(
      fc.property(fc.boolean(), fc.boolean(), (mounted, hasStagedChanges) => {
        const plan = planSync({ mounted, hasStagedChanges });
        if (mounted && hasStagedChanges) {
          expect(plan.action).toBe("commit-and-push");
        } else {
          expect(plan.action).toBe("skip");
        }
      }),
    );
  });
});

describe("Property 15: Vault-Sync Exclusion Correctness", () => {
  // Paths that MUST be excluded (match a .gitignore rule).
  const excludedPaths = [
    "node_modules/foo.js",
    "node_modules/deep/nested/mod.js",
    "src/node_modules/x.js",
    "target/release/bin",
    "build/output.o",
    "dist/bundle.js",
    ".cache/blob",
    "app.log",
    "logs/server.log",
    "deep/dir/trace.log",
    ".idea/workspace.xml",
    ".vscode/settings.json",
    "pkg/__pycache__/mod.pyc",
    ".DS_Store",
    "sub/.DS_Store",
    "scratch.tmp",
    "buffer.swp",
    "shared/sync-file.bin",
    "shared/deep/file.txt",
  ];

  // Paths that MUST be included (no rule matches).
  const includedPaths = [
    "src/main.ts",
    "README.md",
    "docs/architecture.md",
    "package.json",
    "notmodules/file.js", // not node_modules
    "logfile", // not *.log (no dot)
    "my.tmpfile", // not *.tmp
    "shared.txt", // not the shared/ dir
    "src/target.txt", // "target" file, not target/ dir at a path
  ];

  it("all known build/cache/log/junk paths are excluded", () => {
    for (const p of excludedPaths) {
      expect({ path: p, ignored: isIgnored(p, rules) }).toEqual({ path: p, ignored: true });
    }
  });

  it("all source/doc paths are included (not excluded)", () => {
    for (const p of includedPaths) {
      expect({ path: p, ignored: isIgnored(p, rules) }).toEqual({ path: p, ignored: false });
    }
  });

  it("any path under an excluded directory is excluded (random suffixes)", () => {
    const dirRules = ["node_modules", "target", "build", "dist", ".cache", "shared"];
    fc.assert(
      fc.property(
        fc.constantFrom(...dirRules),
        fc.array(fc.stringMatching(/^[a-zA-Z0-9_-]+$/), { minLength: 1, maxLength: 4 }),
        (dir, segments) => {
          const path = `${dir}/${segments.join("/")}`;
          expect(isIgnored(path, rules)).toBe(true);
        },
      ),
    );
  });

  it("any *.log / *.tmp / *.swp file is excluded at any depth", () => {
    fc.assert(
      fc.property(
        fc.array(fc.stringMatching(/^[a-zA-Z0-9_-]+$/), { minLength: 0, maxLength: 3 }),
        fc.stringMatching(/^[a-zA-Z0-9_-]+$/),
        fc.constantFrom("log", "tmp", "swp"),
        (dirs, base, ext) => {
          const prefix = dirs.length ? `${dirs.join("/")}/` : "";
          const path = `${prefix}${base}.${ext}`;
          expect(isIgnored(path, rules)).toBe(true);
        },
      ),
    );
  });

  it("a plain source file with a safe extension is never excluded", () => {
    fc.assert(
      fc.property(
        fc.array(fc.stringMatching(/^[a-zA-Z0-9_-]+$/), { minLength: 1, maxLength: 3 }),
        fc.stringMatching(/^[a-zA-Z0-9_-]+$/),
        fc.constantFrom("ts", "js", "md", "json", "py", "rs", "txt"),
        (dirs, base, ext) => {
          const path = `${dirs.join("/")}/${base}.${ext}`;
          // Guard: skip if a dir segment happens to be an excluded dir name.
          const excludedDirNames = [
            "node_modules",
            "target",
            "build",
            "dist",
            ".cache",
            "shared",
            ".idea",
            ".vscode",
            "__pycache__",
          ];
          fc.pre(!dirs.some((d) => excludedDirNames.includes(d)));
          expect(isIgnored(path, rules)).toBe(false);
        },
      ),
    );
  });
});
