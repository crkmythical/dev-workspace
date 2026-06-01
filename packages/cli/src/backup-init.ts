#!/usr/bin/env bun
/**
 * backup-init — Initialize the restic repository on S3.
 *
 * Run once before the watcher can operate. Safe to re-run: restic reports
 * "already initialized" and exits 0 if the repo exists with matching password.
 *
 * The entrypoint handles auto-init on first boot; this command exists as a
 * manual fallback/diagnostic tool.
 */
import { evaluateBackupGate } from "@sdw/core";
import { $ } from "bun";

const gate = evaluateBackupGate(process.env);
if (gate.status === "dormant") {
  console.error("RESTIC_REPOSITORY not configured. Nothing to initialize.");
  process.exit(1);
}
if (gate.status === "misconfigured") {
  console.error(`Missing credentials: ${gate.missing.join(", ")}`);
  process.exit(1);
}

const proxyUrl = "http://127.0.0.1:7890";
console.log("Initializing restic repository...");

const proc = Bun.spawn(["restic", "init"], {
  stdout: "inherit",
  stderr: "inherit",
  env: {
    ...process.env,
    http_proxy: proxyUrl,
    https_proxy: proxyUrl,
    HTTPS_PROXY: proxyUrl,
    HTTP_PROXY: proxyUrl,
  },
});

const exitCode = await proc.exited;
if (exitCode === 0) {
  console.log("✓ Repository initialized (or already exists).");
} else {
  console.error("✗ Repository initialization failed.");
  process.exit(1);
}
