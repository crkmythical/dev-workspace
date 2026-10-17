import { describe, expect, it } from "bun:test";
import fc from "fast-check";
import {
  CLASH_URL_PLACEHOLDER,
  type EgressReport,
  type EgressStatus,
  describeEgress,
  egressHealth,
  egressNotice,
  egressStatus,
  parseEgressReport,
  renderClashConfig,
  serializeEgressReport,
} from "../src/egress/index.ts";

const report = (status: EgressStatus, providerNodes: number | null = 12): EgressReport => ({
  status,
  checkedAt: "2026-10-17T12:00:00.000Z",
  providerNodes,
});

describe("egressStatus", () => {
  it("maps every observation to exactly one status", () => {
    expect(egressStatus({ proxyListening: true, probeOk: true })).toBe("confirmed");
    expect(egressStatus({ proxyListening: true, probeOk: false })).toBe("degraded");
    expect(egressStatus({ proxyListening: false, probeOk: false })).toBe("down");
  });

  it("a probe cannot succeed through a proxy nobody is listening on: that is still down", () => {
    expect(egressStatus({ proxyListening: false, probeOk: true })).toBe("down");
  });

  it("is confirmed only when BOTH the proxy listens and the probe worked", () => {
    fc.assert(
      fc.property(fc.boolean(), fc.boolean(), (proxyListening, probeOk) => {
        const confirmed = egressStatus({ proxyListening, probeOk }) === "confirmed";
        expect(confirmed).toBe(proxyListening && probeOk);
      }),
    );
  });
});

describe("describeEgress / egressNotice", () => {
  it("explains each state, and shouts when something is wrong", () => {
    expect(describeEgress(report("confirmed"))).toContain("confirmed");
    expect(describeEgress(report("degraded"))).toContain("DEGRADED");
    expect(describeEgress(report("down"))).toContain("DOWN");
  });

  it("points at the subscription when the provider loaded no nodes", () => {
    expect(describeEgress(report("degraded", 0))).toContain("CLASH_SUBSCRIPTION_URL");
    expect(describeEgress(report("degraded", 12))).not.toContain("CLASH_SUBSCRIPTION_URL");
    expect(describeEgress(report("degraded", null))).not.toContain("CLASH_SUBSCRIPTION_URL");
  });

  it("leaves a note only when egress is not confirmed", () => {
    expect(egressNotice(report("confirmed"))).toBeNull();
    const down = egressNotice(report("down")) ?? "";
    expect(down).toContain("# ⚠ Egress DOWN");
    expect(down).toContain("2026-10-17T12:00:00.000Z");
    expect(egressNotice(report("degraded"))).toContain("# ⚠ Egress DEGRADED");
  });

  it("is honest that routing is best effort", () => {
    const note = egressNotice(report("degraded")) ?? "";
    expect(note).toContain("best effort");
    expect(note).toContain("noproxy");
  });
});

describe("the persisted report", () => {
  it("round-trips", () => {
    for (const status of ["confirmed", "degraded", "down"] as const) {
      for (const nodes of [0, 7, null]) {
        const r = report(status, nodes);
        expect(parseEgressReport(serializeEgressReport(r))).toEqual(r);
      }
    }
  });

  it("treats anything it did not write as no report", () => {
    const bad = [
      undefined,
      "",
      "not json",
      "null",
      "[]",
      '{"status":"fine","checkedAt":"x","providerNodes":1}',
      '{"status":"down","providerNodes":1}',
      '{"status":"down","checkedAt":"x"}',
      '{"status":"down","checkedAt":"x","providerNodes":"7"}',
    ];
    for (const raw of bad) expect(parseEgressReport(raw)).toBeNull();
  });
});

describe("egressHealth (the doctor row)", () => {
  it("is ok when confirmed, a warning when degraded, a failure only when down", () => {
    expect(egressHealth("confirmed", null, 7890).status).toBe("ok");
    expect(egressHealth("degraded", null, 7890).status).toBe("warn");
    expect(egressHealth("down", null, 7890).status).toBe("fail");
  });

  it("shows the live state next to what the start-up probe saw", () => {
    const row = egressHealth("confirmed", report("degraded"), 7890);
    expect(row.status).toBe("ok");
    expect(row.detail).toContain("at start-up: degraded");
    expect(egressHealth("confirmed", null, 7890).detail).not.toContain("start-up");
  });

  it("names the port", () => {
    expect(egressHealth("down", null, 7890).detail).toContain(":7890");
  });
});

describe("renderClashConfig", () => {
  const template = `proxy-providers:\n  subscription:\n    url: "${CLASH_URL_PLACEHOLDER}"\n`;

  it("fills the subscription url into the template", () => {
    expect(renderClashConfig(template, "https://sub.example/api?token=abc")).toBe(
      'proxy-providers:\n  subscription:\n    url: "https://sub.example/api?token=abc"\n',
    );
  });

  it("fills every occurrence", () => {
    const twice = `${CLASH_URL_PLACEHOLDER} ${CLASH_URL_PLACEHOLDER}`;
    expect(renderClashConfig(twice, "u")).toBe("u u");
  });

  it("does not interpret replacement patterns inside the url", () => {
    // With a string replacement, `$&` would paste the placeholder back in.
    for (const url of ["https://x/?a=$&b", "https://x/$'y", "https://x/$1$$", "https://x/$`z"]) {
      expect(renderClashConfig(`url: "${CLASH_URL_PLACEHOLDER}"`, url)).toBe(`url: "${url}"`);
    }
  });

  it("escapes what would end the quoted YAML scalar", () => {
    const out = renderClashConfig(`url: "${CLASH_URL_PLACEHOLDER}"`, 'https://x/"q"\\n');
    expect(out).toBe('url: "https://x/\\"q\\"\\\\n"');
  });

  it("keeps a url with a line break on one line", () => {
    const out = renderClashConfig(`url: "${CLASH_URL_PLACEHOLDER}"`, "https://x\nevil: 1");
    expect(out.split("\n")).toHaveLength(1);
  });
});
