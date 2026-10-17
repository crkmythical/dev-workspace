/**
 * Egress use case: start the proxy, wait for it, and observe whether traffic gets out.
 *
 * It never fails. A proxy that does not come up is a finding (status "down"), not an
 * exception: the workspace stays usable and the caller makes the finding visible. The
 * waits are bounded by DEFAULT_EGRESS_TIMING, which is also the worst-case delay this
 * adds to a start-up.
 */
import {
  DEFAULT_EGRESS_TIMING,
  type EgressReport,
  type EgressTiming,
  type Wait,
  describeEgress,
  egressStatus,
} from "./domain.ts";
import type { EgressPorts } from "./ports.ts";

const seconds = (w: Wait) => Math.round((w.attempts * w.delayMs) / 1000);

/** Try up to `attempts` times, sleeping between tries (not after the last). A throwing check is a miss. */
async function waitFor(
  check: () => Promise<boolean>,
  wait: Wait,
  sleep: (ms: number) => Promise<void>,
): Promise<boolean> {
  for (let i = 0; i < wait.attempts; i++) {
    if (await check().catch(() => false)) return true;
    if (i < wait.attempts - 1) await sleep(wait.delayMs);
  }
  return false;
}

/** Wait for the provider to load at least one node; returns the last count seen. */
async function waitForNodes(ports: EgressPorts, wait: Wait): Promise<number | null> {
  let last: number | null = null;
  for (let i = 0; i < wait.attempts; i++) {
    last = await ports.providerNodes().catch(() => null);
    if (last !== null && last > 0) return last;
    if (i < wait.attempts - 1) await ports.sleep(wait.delayMs);
  }
  return last;
}

export async function bringUpEgress(
  ports: EgressPorts,
  timing: EgressTiming = DEFAULT_EGRESS_TIMING,
): Promise<EgressReport> {
  ports.log("Starting Clash...");
  try {
    await ports.startProxy();
  } catch (err) {
    // Maybe it is already running; the wait below finds out.
    ports.log(`WARNING: could not start Clash: ${err instanceof Error ? err.message : err}`);
  }

  const listening = await waitFor(() => ports.proxyListening(), timing.proxyWait, ports.sleep);
  ports.log(
    listening ? "Clash ready." : `WARNING: Clash not ready after ${seconds(timing.proxyWait)}s.`,
  );

  let providerNodes: number | null = null;
  let probeOk = false;
  if (listening) {
    providerNodes = await waitForNodes(ports, timing.providerWait);
    if (!(providerNodes !== null && providerNodes > 0)) {
      // The start-up fetch may simply have lost a race with the network; Clash would not try
      // again for an hour, so ask once, now.
      ports.log(
        `Provider has no nodes after ${seconds(timing.providerWait)}s; asking Clash to fetch the subscription again.`,
      );
      await ports.refreshProvider().catch(() => {});
      providerNodes = await waitForNodes(ports, timing.refreshWait);
    }
    ports.log(
      providerNodes !== null && providerNodes > 0
        ? `Provider loaded: ${providerNodes} nodes (url-test auto-selecting fastest).`
        : "WARNING: Provider still has no nodes (Clash will retry on its own schedule).",
    );

    await ports.selectAutoNode().catch(() => {});

    // Give Clash a moment to establish its first connection before probing.
    await ports.sleep(timing.settleMs);
    probeOk = await waitFor(() => ports.probe(), timing.probe, ports.sleep);
  }

  const report: EgressReport = {
    status: egressStatus({ proxyListening: listening, probeOk }),
    checkedAt: ports.now(),
    providerNodes,
  };
  ports.log(
    report.status === "confirmed"
      ? `Egress ${describeEgress(report)}.`
      : `WARNING: egress ${describeEgress(report)}`,
  );
  return report;
}
