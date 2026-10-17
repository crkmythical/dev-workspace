/**
 * Launchers for the cli's commands, generated from package.json's `bin` map so that
 * map is the only list of commands (the image used to repeat it in the Dockerfile, and
 * the two had drifted: `backup-init` was installed but never declared).
 */
import { chmodSync, mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";

export type BinMap = Readonly<Record<string, string>>;

/** The shell wrapper that runs one command's entry file with bun. */
export function launcher(entryFile: string): string {
  return `#!/bin/bash\nexec bun run ${entryFile} "$@"\n`;
}

/** Write one executable launcher per command into `destDir`. Returns the command names. */
export function installBins(cliRoot: string, bin: BinMap, destDir: string): string[] {
  mkdirSync(destDir, { recursive: true });
  const names = Object.keys(bin).sort();
  for (const name of names) {
    const target = path.join(destDir, name);
    writeFileSync(target, launcher(path.resolve(cliRoot, bin[name])));
    chmodSync(target, 0o755);
  }
  return names;
}
