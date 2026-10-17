#!/usr/bin/env bun
/**
 * self-destruct — Emergency data destruction CLI command.
 *
 * Usage: self-destruct [--force] [--skip-shred] [--remote]
 */
import { destroyWorkspace } from "@sdw/core/vault";
import { createDestroyDeps } from "@sdw/infra";
import { $ } from "bun";

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

const report = await destroyWorkspace(createDestroyDeps(console.log), { skipShred, remote });

if (!report.complete) {
  console.error("Destroy finished with failures:");
  for (const p of report.phases.filter((phase) => !phase.ok)) {
    console.error(`  ${p.phase}: ${p.detail ?? "failed"}`);
  }
}

// Post-destroy: stop all services then kill PID 1 (container exits).
await $`supervisorctl stop all`.quiet().nothrow();
await $`kill -TERM 1`.quiet().nothrow();
// tini propagates SIGTERM to all children including this process — expected.
