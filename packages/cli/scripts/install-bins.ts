#!/usr/bin/env bun
/**
 * Image build step: put a launcher for every command of package.json's `bin` map on
 * PATH.   bun run packages/cli/scripts/install-bins.ts [/usr/local/bin]
 */
import path from "node:path";
import { installBins } from "../src/lib/bins.ts";

const cliRoot = path.resolve(import.meta.dir, "..");
const destDir = process.argv[2] ?? "/usr/local/bin";
const { bin } = (await Bun.file(path.join(cliRoot, "package.json")).json()) as {
  bin: Record<string, string>;
};

const names = installBins(cliRoot, bin, destDir);
console.log(`Installed ${names.length} commands into ${destDir}: ${names.join(", ")}`);
