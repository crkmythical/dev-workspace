/**
 * supervisord program files: switching on-demand programs to start at boot.
 * Replaces `sed -i` one-liners with something that can be tested against a temp file.
 */
import { existsSync, readFileSync, writeFileSync } from "node:fs";

/** `autostart=false` -> `autostart=true`, on directive lines only (not in comments). */
export function withAutostart(conf: string): string {
  return conf.replace(/^(\s*autostart\s*=\s*)false\b/gm, "$1true");
}

/**
 * Make supervisord start the programs in `file` by itself. A missing file is skipped:
 * not every image contains every desktop stack. Returns whether the file changed.
 */
export function enableAutostart(file: string): boolean {
  if (!existsSync(file)) return false;
  const before = readFileSync(file, "utf-8");
  const after = withAutostart(before);
  if (after === before) return false;
  writeFileSync(file, after);
  return true;
}
