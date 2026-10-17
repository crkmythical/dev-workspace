/**
 * Egress adapters: the Clash/mihomo implementation of EgressPorts, a quick live
 * observation for `doctor`, and the two places the start-up result is left for
 * people to find (a status file and a note inside the vault).
 *
 * What counts as "confirmed", "degraded" and "down" is decided in @sdw/core/egress.
 */
import { readFileSync, writeFileSync } from "node:fs";
import {
  CLASH_AUTO_GROUP,
  CLASH_BIN,
  CLASH_CONTROLLER_PORT,
  CLASH_DIR,
  CLASH_HTTP_PORT,
  CLASH_PROVIDER_NAME,
  EGRESS_NOTIFICATION_PATH,
  EGRESS_PROBE_URL,
  EGRESS_STATUS_PATH,
  type EgressObservation,
  type EgressPorts,
  type EgressReport,
  egressNotice,
  parseEgressReport,
  serializeEgressReport,
} from "@sdw/core/egress";
import { $ } from "bun";
import { removeNote, writeNote } from "./notifications.ts";

const PROXY_URL = `http://127.0.0.1:${CLASH_HTTP_PORT}`;
const CONTROLLER_URL = `http://127.0.0.1:${CLASH_CONTROLLER_PORT}`;

async function portAccepts(port: number): Promise<boolean> {
  const r = await $`nc -z 127.0.0.1 ${port}`.quiet().nothrow();
  return r.exitCode === 0;
}

export function clashEgressPorts(log: (message: string) => void): EgressPorts {
  return {
    async startProxy() {
      Bun.spawn([CLASH_BIN, "-d", CLASH_DIR], { stdout: "ignore", stderr: "ignore" });
    },

    proxyListening: () => portAccepts(CLASH_HTTP_PORT),

    async providerNodes() {
      try {
        const resp = await fetch(`${CONTROLLER_URL}/providers/proxies/${CLASH_PROVIDER_NAME}`);
        if (!resp.ok) return null;
        const data = (await resp.json()) as { proxies?: Record<string, unknown> };
        return Object.keys(data.proxies ?? {}).length;
      } catch {
        return null;
      }
    },

    async selectAutoNode() {
      await fetch(`${CONTROLLER_URL}/proxies/GLOBAL`, {
        method: "PUT",
        body: JSON.stringify({ name: CLASH_AUTO_GROUP }),
        headers: { "Content-Type": "application/json" },
      });
    },

    async probe() {
      const r =
        await $`curl -fsSL --proxy ${PROXY_URL} --max-time 10 ${EGRESS_PROBE_URL} -o /dev/null`
          .quiet()
          .nothrow();
      return r.exitCode === 0;
    },

    sleep: (ms) => Bun.sleep(ms),
    now: () => new Date().toISOString(),
    log,
  };
}

/**
 * A fast look at the state right now (seconds, not the start-up's patient waits).
 * `-f` makes an HTTP error from the proxy count as a failed probe.
 */
export async function observeEgress(): Promise<EgressObservation> {
  const proxyListening = await portAccepts(CLASH_HTTP_PORT);
  if (!proxyListening) return { proxyListening, probeOk: false };
  const r =
    await $`curl -sf --connect-timeout 2 --max-time 5 -x ${PROXY_URL} http://www.gstatic.com/generate_204 -o /dev/null`
      .quiet()
      .nothrow();
  return { proxyListening, probeOk: r.exitCode === 0 };
}

// --- where the start-up result is left ---

export function writeEgressStatus(report: EgressReport, file: string = EGRESS_STATUS_PATH): void {
  writeFileSync(file, serializeEgressReport(report));
}

export function readEgressStatus(file: string = EGRESS_STATUS_PATH): EgressReport | null {
  try {
    return parseEgressReport(readFileSync(file, "utf-8"));
  } catch {
    return null;
  }
}

/**
 * Make the note inside the vault match the report: written while egress is not
 * confirmed, removed once it is. "unavailable" when the vault is not mounted.
 */
export function syncEgressNotice(
  report: EgressReport,
  file: string = EGRESS_NOTIFICATION_PATH,
): "written" | "removed" | "unavailable" {
  const note = egressNotice(report);
  if (note === null) {
    removeNote(file);
    return "removed";
  }
  return writeNote(file, note) ? "written" : "unavailable";
}
