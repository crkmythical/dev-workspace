#!/usr/bin/env bun
/**
 * push — send files out of the workspace with croc (end-to-end encrypted).
 *
 *   push [--direct] [--code CODE] <path> [path...]
 *
 * Through the self-hosted relay in CROC_RELAY, by default via Clash (--socks5), so
 * the relay sees only ciphertext; `--direct` is for a relay on the LAN. CROC_PASS,
 * if set, authenticates to the relay. Without --code, croc prints a generated phrase
 * to hand to the receiver.
 */
import { buildCrocSend } from "@sdw/core/transfer";

const relay = process.env.CROC_RELAY;
if (!relay) {
  console.error(
    "File transfer is off. Set CROC_RELAY (and CROC_PASS) to a self-hosted croc relay.",
  );
  process.exit(1);
}

const argv = process.argv.slice(2);
let direct = false;
let code: string | undefined;
const paths: string[] = [];
for (let i = 0; i < argv.length; i++) {
  const a = argv[i];
  if (a === "--direct") direct = true;
  else if (a === "--code") code = argv[++i];
  else paths.push(a);
}
if (paths.length === 0) {
  console.error("Usage: push [--direct] [--code CODE] <path> [path...]");
  process.exit(1);
}

// Secrets go in the environment, never on the command line: CROC_PASS (relay) and, for a
// chosen code phrase, CROC_SECRET (croc v11 refuses `send --code`).
const env = { ...process.env };
if (process.env.CROC_PASS) env.CROC_PASS = process.env.CROC_PASS;
if (code) env.CROC_SECRET = code;

const proc = Bun.spawn(
  ["croc", ...buildCrocSend({ relay, socks: direct ? null : undefined, paths })],
  {
    stdin: "inherit",
    stdout: "inherit",
    stderr: "inherit",
    env,
  },
);
process.exit(await proc.exited);
