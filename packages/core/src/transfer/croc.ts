/**
 * Pure builders for the croc command line. No environment, no spawning: the cli
 * `push` / `pull` adapters read the environment and run `croc` with these.
 *
 * croc is end-to-end encrypted (PAKE from the code phrase, then AES-256-GCM); the
 * relay only ever sees ciphertext. Global flags (--relay, --socks5) come before
 * the subcommand.
 *
 * Neither secret is ever an argument: croc reads the relay password from CROC_PASS
 * and the code phrase from CROC_SECRET (croc v11 refuses `send --code` on the
 * command line for exactly this reason). The cli adapters put them in the child's
 * environment; these builders produce only the non-secret argv.
 */
import { CLASH_SOCKS_ADDR } from "./layout.ts";

export interface SendOptions {
  /** Self-hosted relay, host:port. */
  relay: string;
  /**
   * SOCKS5 proxy for the relay connection. `undefined` means "use Clash" (the
   * default, for a remote relay); `null` means go direct (a LAN/reachable relay).
   */
  socks?: string | null;
  paths: readonly string[];
}

export interface ReceiveOptions {
  relay: string;
  socks?: string | null;
  /** Directory received files are written to. */
  out: string;
}

function globals(relay: string, socks: string | null | undefined): string[] {
  const args = ["--relay", relay];
  const proxy = socks === undefined ? CLASH_SOCKS_ADDR : socks;
  if (proxy) args.push("--socks5", proxy);
  return args;
}

export function buildCrocSend(o: SendOptions): string[] {
  const args = globals(o.relay, o.socks);
  args.push("send", ...o.paths);
  return args;
}

export function buildCrocReceive(o: ReceiveOptions): string[] {
  const args = globals(o.relay, o.socks);
  // --yes: no interactive "accept?" prompt (push/pull run scripted). The code comes
  // from CROC_SECRET, so there is no positional argument here.
  args.push("--yes", "--out", o.out);
  return args;
}
