/**
 * Path-safety primitives for untrusted, post-decryption file paths.
 *
 * Even though sync payloads are AEAD-authenticated (an attacker must hold the
 * vault key to produce a valid envelope), the decrypted `file_path` /
 * download `path` is still attacker-influenced data. Defense-in-depth: a path
 * supplied by the client must never escape the shared directory via `..`,
 * absolute paths, or NUL injection.
 *
 * Pure logic (no IO) so the traversal invariant is property-testable.
 */

/** Reasons a candidate path is rejected. */
export type PathRejection = "empty" | "absolute" | "nul-byte" | "traversal" | "backslash";

export type PathCheck = { ok: true; normalized: string } | { ok: false; reason: PathRejection };

/**
 * Validate a relative file path that will be joined under a trusted base dir.
 *
 * Accepts only paths that stay strictly inside the base when resolved:
 *   - non-empty
 *   - no NUL bytes
 *   - not absolute (no leading "/")
 *   - no backslashes (avoid Windows-style separators smuggling segments)
 *   - no "." / ".." segment that would climb out
 *
 * Returns the POSIX-normalized relative path on success.
 */
export function checkRelativePath(input: string): PathCheck {
  if (input.length === 0) return { ok: false, reason: "empty" };
  if (input.includes("\0")) return { ok: false, reason: "nul-byte" };
  if (input.includes("\\")) return { ok: false, reason: "backslash" };
  if (input.startsWith("/")) return { ok: false, reason: "absolute" };

  const segments = input.split("/");
  const out: string[] = [];
  for (const seg of segments) {
    if (seg === "" || seg === ".") continue; // collapse empty / current-dir
    if (seg === "..") return { ok: false, reason: "traversal" };
    out.push(seg);
  }
  if (out.length === 0) return { ok: false, reason: "empty" };

  return { ok: true, normalized: out.join("/") };
}

/** Convenience boolean form. */
export function isSafeRelativePath(input: string): boolean {
  return checkRelativePath(input).ok;
}
