#!/usr/bin/env bun
/**
 * self-destruct — Emergency data destruction CLI command.
 *
 * Usage: self-destruct [--force] [--skip-shred] [--remote]
 */
import { $ } from "bun";
import { destroyCore } from "./lib/destroy.ts";

const args = new Set(process.argv.slice(2));
const force = args.has("--force");
const skipShred = args.has("--skip-shred");
const remote = args.has("--remote");

if (!force) {
  const answer = prompt("Type 'destroy' to confirm: ");
  if (answer !== "destroy") {
    console.log("Aborted.");
    process.exit(0);
  }
}

await destroyCore({ force: true, skipShred, remote, silent: false });

// Post-destroy: stop all services then kill PID 1 (container exits).
await $`supervisorctl stop all`.quiet().nothrow();
await $`kill -TERM 1`.quiet().nothrow();
// tini propagates SIGTERM to all children including this process — expected.
