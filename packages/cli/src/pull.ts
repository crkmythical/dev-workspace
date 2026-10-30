#!/usr/bin/env bun
/**
 * pull — receive files into the workspace with croc.
 *
 *   pull [--direct] [--out DIR] <code>
 *
 * <code> is the phrase `push` printed on the other side. Files land in DIR
 * (default: the vault mount). Same relay and proxy rules as `push`.
 */
import { TRANSFER_DEFAULT_OUT, buildCrocReceive } from "@sdw/core/transfer";

const relay = process.env.CROC_RELAY;
if (!relay) {
  console.error(
    "File transfer is off. Set CROC_RELAY (and CROC_PASS) to a self-hosted croc relay.",
  );
  process.exit(1);
}

const argv = process.argv.slice(2);
let direct = false;
let out = TRANSFER_DEFAULT_OUT;
let code: string | undefined;
for (let i = 0; i < argv.length; i++) {
  const a = argv[i];
  if (a === "--direct") direct = true;
  else if (a === "--out") out = argv[++i];
  else code = a;
}
if (!code) {
  console.error("Usage: pull [--direct] [--out DIR] <code>");
  process.exit(1);
}

// The code phrase goes in CROC_SECRET, not on the command line; CROC_PASS (if set)
// authenticates to the relay.
const env = { ...process.env };
env.CROC_SECRET = code;
if (process.env.CROC_PASS) env.CROC_PASS = process.env.CROC_PASS;

const proc = Bun.spawn(
  ["croc", ...buildCrocReceive({ relay, socks: direct ? null : undefined, out })],
  {
    stdin: "inherit",
    stdout: "inherit",
    stderr: "inherit",
    env,
  },
);
process.exit(await proc.exited);
