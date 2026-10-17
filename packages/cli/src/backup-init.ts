#!/usr/bin/env bun
/**
 * backup-init — Initialize the restic repository on S3.
 *
 * Safe to re-run: the repository is probed first, and only a location where restic says
 * there is no repository gets `restic init`. (Calling `restic init` on an existing repository
 * exits 1, which this command used to report as a failed initialization.)
 *
 * The entrypoint handles auto-init on first boot; this command exists as a
 * manual fallback/diagnostic tool.
 */
import { evaluateBackupGate } from "@sdw/core/backup";
import { loadConfig } from "@sdw/core/workspace";
import { initRepository, probeRepository } from "@sdw/infra";

const gate = evaluateBackupGate(loadConfig(process.env).backup);
if (gate.status === "dormant") {
  console.error("RESTIC_REPOSITORY not configured. Nothing to initialize.");
  process.exit(1);
}
if (gate.status === "misconfigured") {
  console.error(`Missing credentials: ${gate.missing.join(", ")}`);
  process.exit(1);
}

const { verdict, detail } = await probeRepository(process.env);
switch (verdict) {
  case "ready":
    console.log("✓ Repository already initialized and readable with this password.");
    break;
  case "wrong-password":
    console.error("✗ A repository exists at that location, but RESTIC_PASSWORD does not open it.");
    process.exit(1);
    break;
  case "unreachable":
    console.error("✗ Could not reach the repository (network or credentials?). Not initializing.");
    if (detail) console.error(`  ${detail.split("\n").slice(-1)[0]}`);
    process.exit(1);
    break;
  case "missing":
    console.log("Initializing restic repository...");
    if (await initRepository(process.env)) {
      console.log("✓ Repository initialized.");
    } else {
      console.error("✗ Repository initialization failed.");
      process.exit(1);
    }
    break;
}
