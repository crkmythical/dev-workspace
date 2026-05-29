/**
 * Minimal gitignore pattern matcher — pure, no filesystem access.
 *
 * Supports the subset of gitignore syntax used by the vault's baked
 * `.gitignore` (see init-vault.ts):
 *   - directory patterns ending in `/`        e.g. `node_modules/`, `shared/`
 *   - glob patterns with star and double-star  e.g. "*.log", "(double-star)/*.log"
 *   - plain names / extensions                 e.g. `.DS_Store`, `*.tmp`
 *
 * Used by Property 15 (Vault-Sync Exclusion Correctness) to assert that the
 * exclusion set matches the intent of git's own `check-ignore` for these
 * patterns. The real `vault-sync` defers to git itself; this module exists so
 * the exclusion intent is independently verifiable.
 */

export interface GitignoreRule {
  /** The raw pattern from the .gitignore file. */
  pattern: string;
  /** True if pattern targets directories only (trailing slash). */
  dirOnly: boolean;
  /** True if pattern is anchored to the repo root (leading slash). */
  anchored: boolean;
  regex: RegExp;
}

/** Translate a single gitignore glob segment-body into a regex source string. */
function globToRegexSource(glob: string): string {
  let src = "";
  for (let i = 0; i < glob.length; i++) {
    const c = glob[i];
    if (c === "*") {
      if (glob[i + 1] === "*") {
        // `**` matches across path separators
        src += ".*";
        i++;
        // consume a following slash so `**/` collapses cleanly
        if (glob[i + 1] === "/") i++;
      } else {
        // single `*` matches anything except a slash
        src += "[^/]*";
      }
    } else if (c === "?") {
      src += "[^/]";
    } else if (".+^${}()|[]\\".includes(c)) {
      src += `\\${c}`;
    } else {
      src += c;
    }
  }
  return src;
}

/** Parse one non-empty, non-comment gitignore line into a rule. */
export function parseRule(rawLine: string): GitignoreRule | null {
  const line = rawLine.trim();
  if (line === "" || line.startsWith("#")) return null;

  let pattern = line;
  const dirOnly = pattern.endsWith("/");
  if (dirOnly) pattern = pattern.slice(0, -1);

  const anchored = pattern.startsWith("/");
  if (anchored) pattern = pattern.slice(1);

  const bodyHasSlash = pattern.includes("/");
  const body = globToRegexSource(pattern);

  // Anchored or slash-containing patterns match from the repo root.
  // Bare names (no slash) match in any directory.
  const prefix = anchored || bodyHasSlash ? "^" : "(^|.*/)";
  // A matched path is the entry itself or anything beneath it.
  const suffix = "(/.*)?$";
  const regex = new RegExp(`${prefix}${body}${suffix}`);

  return { pattern: line, dirOnly, anchored, regex };
}

/** Parse a full .gitignore file body into rules. */
export function parseGitignore(content: string): GitignoreRule[] {
  return content
    .split("\n")
    .map(parseRule)
    .filter((r): r is GitignoreRule => r !== null);
}

/**
 * Returns true if `path` (a repo-relative POSIX path, no leading slash) is
 * ignored by any rule. `isDir` indicates whether the path is a directory,
 * which matters for directory-only (`foo/`) patterns.
 */
export function isIgnored(path: string, rules: GitignoreRule[], isDir = false): boolean {
  const normalized = path.replace(/^\/+/, "");
  for (const rule of rules) {
    if (rule.dirOnly) {
      // A directory-only rule matches the directory itself (if isDir) or any
      // descendant path under that directory.
      if (rule.regex.test(normalized)) {
        // For dir-only patterns, the match must be the dir or something under it.
        // The regex already encodes `(/.*)?$`, so a file directly named like the
        // pattern won't be excluded unless a descendant relationship exists.
        if (isDir || normalized.includes("/") || hasDescendantMatch(normalized, rule)) {
          return true;
        }
      }
    } else if (rule.regex.test(normalized)) {
      return true;
    }
  }
  return false;
}

/** Whether the path lies *under* a directory matched by a dir-only rule. */
function hasDescendantMatch(path: string, rule: GitignoreRule): boolean {
  const segments = path.split("/");
  // Build progressive prefixes: a/b/c -> "a", "a/b", "a/b/c"
  let prefix = "";
  for (const seg of segments) {
    prefix = prefix ? `${prefix}/${seg}` : seg;
    if (rule.regex.test(prefix) && path.length > prefix.length) {
      return true;
    }
  }
  return false;
}
