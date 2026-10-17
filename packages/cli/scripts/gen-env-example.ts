#!/usr/bin/env bun
/**
 * Regenerate `.env.example` from the env schema (core, plus each plugin's variables).
 *
 *   bun run packages/cli/scripts/gen-env-example.ts           # write it
 *   bun run packages/cli/scripts/gen-env-example.ts --check   # exit 1 if it is out of date
 */
import path from "node:path";
import { ENV_GROUPS } from "@sdw/core/plugins";
import { renderEnvExample } from "@sdw/core/workspace";

const target = path.resolve(import.meta.dir, "../../../.env.example");
const text = renderEnvExample(ENV_GROUPS);

if (process.argv.includes("--check")) {
  const current = (await Bun.file(target).exists()) ? await Bun.file(target).text() : "";
  if (current !== text) {
    console.error(".env.example is out of date. Run: make env-example");
    process.exit(1);
  }
  console.log(".env.example is up to date.");
} else {
  await Bun.write(target, text);
  console.log(`Wrote ${target}`);
}
