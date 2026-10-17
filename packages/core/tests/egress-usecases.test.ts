import { describe, expect, it } from "bun:test";
import {
  DEFAULT_EGRESS_TIMING,
  type EgressPorts,
  bringUpEgress,
  describeEgress,
} from "../src/egress/index.ts";

/**
 * bringUpEgress against a fake proxy and a fake clock. `sleep` only advances a counter,
 * so the worst-case delay a start-up can spend waiting is asserted, not experienced.
 */
interface Scenario {
  /** Attempt number (1-based) at which the proxy starts listening; Infinity = never. */
  listensAtAttempt?: number;
  /** Node counts returned per call; the last one repeats. null = controller unreachable. */
  nodes?: (number | null)[];
  /** Attempt number (1-based) at which the probe starts succeeding; Infinity = never. */
  probeOkAtAttempt?: number;
  startThrows?: boolean;
  selectThrows?: boolean;
}

function fakePorts(s: Scenario = {}) {
  const calls: string[] = [];
  const logs: string[] = [];
  let elapsedMs = 0;
  let listenChecks = 0;
  let nodeChecks = 0;
  let probes = 0;
  const nodes = s.nodes ?? [12];

  const ports: EgressPorts = {
    async startProxy() {
      calls.push("start");
      if (s.startThrows) throw new Error("clash: not found");
    },
    async proxyListening() {
      listenChecks++;
      calls.push("listening?");
      return listenChecks >= (s.listensAtAttempt ?? 1);
    },
    async providerNodes() {
      calls.push("nodes?");
      return nodes[Math.min(nodeChecks++, nodes.length - 1)];
    },
    async selectAutoNode() {
      calls.push("select");
      if (s.selectThrows) throw new Error("controller refused");
    },
    async probe() {
      probes++;
      calls.push("probe");
      return probes >= (s.probeOkAtAttempt ?? 1);
    },
    async sleep(ms) {
      elapsedMs += ms;
    },
    now: () => "2026-10-17T12:00:00.000Z",
    log: (m) => logs.push(m),
  };
  return {
    ports,
    calls,
    logs,
    elapsed: () => elapsedMs,
    count: (name: string) => calls.filter((c) => c === name).length,
  };
}

const T = DEFAULT_EGRESS_TIMING;

describe("bringUpEgress", () => {
  it("a healthy proxy is confirmed after one settle pause", async () => {
    const f = fakePorts();
    const report = await bringUpEgress(f.ports);
    expect(report).toEqual({
      status: "confirmed",
      checkedAt: "2026-10-17T12:00:00.000Z",
      providerNodes: 12,
    });
    // The order matters: the auto group is selected before the first probe.
    expect(f.calls).toEqual(["start", "listening?", "nodes?", "select", "probe"]);
    expect(f.elapsed()).toBe(T.settleMs);
    expect(f.logs).toContain("Clash ready.");
    expect(f.logs.at(-1)).toContain("Egress confirmed");
  });

  it("a proxy that never listens is DOWN, without probing or selecting anything", async () => {
    const f = fakePorts({ listensAtAttempt: Number.POSITIVE_INFINITY });
    const report = await bringUpEgress(f.ports);
    expect(report.status).toBe("down");
    expect(report.providerNodes).toBeNull();
    expect(f.count("listening?")).toBe(T.proxyWait.attempts);
    expect(f.calls).not.toContain("nodes?");
    expect(f.calls).not.toContain("select");
    expect(f.calls).not.toContain("probe");
    // The most a dead proxy can delay a start-up: 29 gaps of one second, nothing after.
    expect(f.elapsed()).toBe((T.proxyWait.attempts - 1) * T.proxyWait.delayMs);
    expect(f.logs.some((l) => l.includes("WARNING: Clash not ready after 30s"))).toBe(true);
    expect(f.logs.at(-1)).toContain("WARNING: egress DOWN");
  });

  it("a proxy that comes up late is waited for, not given up on", async () => {
    const f = fakePorts({ listensAtAttempt: 5 });
    const report = await bringUpEgress(f.ports);
    expect(report.status).toBe("confirmed");
    expect(f.count("listening?")).toBe(5);
    expect(f.elapsed()).toBe(4 * T.proxyWait.delayMs + T.settleMs);
  });

  it("a proxy that listens but cannot reach out is DEGRADED after exactly the probe budget", async () => {
    const f = fakePorts({ probeOkAtAttempt: Number.POSITIVE_INFINITY });
    const report = await bringUpEgress(f.ports);
    expect(report.status).toBe("degraded");
    expect(f.count("probe")).toBe(T.probe.attempts);
    expect(f.elapsed()).toBe(T.settleMs + (T.probe.attempts - 1) * T.probe.delayMs);
    expect(f.logs.at(-1)).toContain("WARNING: egress DEGRADED");
  });

  it("a flaky first probe is retried", async () => {
    const f = fakePorts({ probeOkAtAttempt: 2 });
    expect((await bringUpEgress(f.ports)).status).toBe("confirmed");
    expect(f.count("probe")).toBe(2);
  });

  it("an empty subscription is recorded, and still probed", async () => {
    const f = fakePorts({ nodes: [0], probeOkAtAttempt: Number.POSITIVE_INFINITY });
    const report = await bringUpEgress(f.ports);
    expect(report.status).toBe("degraded");
    expect(report.providerNodes).toBe(0);
    expect(f.count("nodes?")).toBe(T.providerWait.attempts);
    expect(describeEgress(report)).toContain("CLASH_SUBSCRIPTION_URL");
    expect(f.logs.some((l) => l.includes("Provider did not load nodes within 30s"))).toBe(true);
  });

  it("nodes that load after a few polls stop the wait early", async () => {
    const f = fakePorts({ nodes: [null, 0, 0, 9] });
    const report = await bringUpEgress(f.ports);
    expect(report.providerNodes).toBe(9);
    expect(f.count("nodes?")).toBe(4);
    expect(f.logs).toContain("Provider loaded: 9 nodes (url-test auto-selecting fastest).");
  });

  it("an unreachable controller leaves the node count unknown", async () => {
    const f = fakePorts({ nodes: [null] });
    const report = await bringUpEgress(f.ports);
    expect(report.providerNodes).toBeNull();
    expect(report.status).toBe("confirmed");
  });

  it("failing to launch the proxy is logged, and the wait still decides the outcome", async () => {
    const dead = fakePorts({ startThrows: true, listensAtAttempt: Number.POSITIVE_INFINITY });
    expect((await bringUpEgress(dead.ports)).status).toBe("down");
    expect(dead.logs.some((l) => l.includes("could not start Clash: clash: not found"))).toBe(true);

    // ... for instance when a proxy from an earlier run is already listening.
    const alreadyUp = fakePorts({ startThrows: true });
    expect((await bringUpEgress(alreadyUp.ports)).status).toBe("confirmed");
  });

  it("a refused auto-select does not abort the bring-up", async () => {
    const f = fakePorts({ selectThrows: true });
    expect((await bringUpEgress(f.ports)).status).toBe("confirmed");
    expect(f.calls).toContain("probe");
  });

  it("never throws, even when every port misbehaves", async () => {
    const boom = async () => {
      throw new Error("boom");
    };
    const ports: EgressPorts = {
      startProxy: boom,
      proxyListening: boom,
      providerNodes: boom,
      selectAutoNode: boom,
      probe: boom,
      sleep: async () => {},
      now: () => "t",
      log: () => {},
    };
    expect((await bringUpEgress(ports)).status).toBe("down");
  });

  it("honors a custom timing (the budget is data, not a constant in the loop)", async () => {
    const f = fakePorts({ listensAtAttempt: Number.POSITIVE_INFINITY });
    await bringUpEgress(f.ports, { ...T, proxyWait: { attempts: 3, delayMs: 10 } });
    expect(f.count("listening?")).toBe(3);
    expect(f.elapsed()).toBe(20);
  });
});
