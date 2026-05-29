#!/usr/bin/env bun
/**
 * desktop-stop — Stop the remote desktop session.
 */
import { $ } from "bun";

await $`supervisorctl stop desktop:*`.quiet().nothrow();

// Confirm all desktop processes stopped
const status = await $`supervisorctl status desktop:*`.quiet().nothrow();
const lines = status
  .text()
  .split("\n")
  .filter((l) => l.includes("RUNNING"));
if (lines.length > 0) {
  console.warn("⚠ Some desktop processes still running:");
  for (const l of lines) console.warn(`  ${l}`);
} else {
  console.log("Desktop stopped.");
}
