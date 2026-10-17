/**
 * Egress domain: what we know about the container's route to the internet.
 *
 * Egress is BEST EFFORT, not enforced. The image points http_proxy / https_proxy at
 * Clash, so every proxy-aware tool goes through it, but nothing stops a process that
 * ignores those variables (or runs under `noproxy <cmd>`) from connecting directly:
 * there is no firewall rule. This module therefore reports what was OBSERVED, never
 * what is guaranteed. Pure; the probing lives in @sdw/infra.
 */

/**
 *   confirmed  the proxy is listening and a request through it worked
 *   degraded   the proxy is listening but a request through it failed (nodes down, or
 *              the subscription has not finished loading)
 *   down       nothing is listening on the proxy port
 */
export type EgressStatus = "confirmed" | "degraded" | "down";

export interface EgressObservation {
  /** Something accepts connections on the proxy port. */
  proxyListening: boolean;
  /** A request to the probe target, made through the proxy, succeeded. */
  probeOk: boolean;
}

export function egressStatus(o: EgressObservation): EgressStatus {
  // A probe "through" a proxy nobody is listening on cannot have worked.
  if (!o.proxyListening) return "down";
  return o.probeOk ? "confirmed" : "degraded";
}

export interface EgressReport {
  status: EgressStatus;
  /** ISO timestamp of the observation. */
  checkedAt: string;
  /** Nodes the subscription provider had loaded; null when the controller did not answer. */
  providerNodes: number | null;
}

/** One line for logs and the banner. Non-confirmed states are upper-case on purpose. */
export function describeEgress(report: EgressReport): string {
  switch (report.status) {
    case "confirmed":
      return "confirmed (a request through the proxy succeeded)";
    case "degraded":
      return report.providerNodes === 0
        ? "DEGRADED: the proxy is up but its subscription loaded no nodes (check CLASH_SUBSCRIPTION_URL)"
        : "DEGRADED: the proxy is up but a request through it failed (nodes may be down or still starting)";
    case "down":
      return "DOWN: nothing is listening on the proxy port";
  }
}

/** The note left for the user while egress is not confirmed; null when there is nothing to say. */
export function egressNotice(report: EgressReport): string | null {
  if (report.status === "confirmed") return null;
  return [
    `# ⚠ Egress ${report.status === "down" ? "DOWN" : "DEGRADED"}`,
    "",
    `Checked at ${report.checkedAt}.`,
    "",
    describeEgress(report),
    "",
    "Tools that honor `http_proxy` (git, curl, npm, restic) will fail until the proxy recovers.",
    "The workspace itself keeps running.",
    "",
    "Egress is best effort: nothing blocks a process that ignores the proxy settings (or runs",
    "under `noproxy <cmd>`) from connecting directly.",
    "",
    "Run `doctor` for the live state. This note is removed at the next start once egress is confirmed.",
    "",
  ].join("\n");
}

// --- persisted form (what `doctor` reads back) ---

export function serializeEgressReport(report: EgressReport): string {
  return `${JSON.stringify(report)}\n`;
}

const STATUSES: readonly string[] = ["confirmed", "degraded", "down"];

/** Null for anything that is not a report we wrote: the file is only a convenience. */
export function parseEgressReport(raw: string | undefined): EgressReport | null {
  if (!raw) return null;
  let data: unknown;
  try {
    data = JSON.parse(raw);
  } catch {
    return null;
  }
  if (typeof data !== "object" || data === null) return null;
  const r = data as Record<string, unknown>;
  if (typeof r.status !== "string" || !STATUSES.includes(r.status)) return null;
  if (typeof r.checkedAt !== "string") return null;
  const nodes = r.providerNodes;
  if (nodes !== null && !(typeof nodes === "number" && Number.isFinite(nodes))) return null;
  return { status: r.status as EgressStatus, checkedAt: r.checkedAt, providerNodes: nodes };
}

// --- doctor ---

/**
 * The doctor row: the LIVE state, plus what the start-up probe saw. A degraded
 * proxy is a warning (it is often just nodes still loading); only "nothing is
 * listening" is a failure.
 */
export function egressHealth(
  live: EgressStatus,
  atBoot: EgressReport | null,
  proxyPort: number,
): { status: "ok" | "warn" | "fail"; detail: string } {
  const boot = atBoot ? `; at start-up: ${atBoot.status}` : "";
  switch (live) {
    case "confirmed":
      return { status: "ok", detail: `via 127.0.0.1:${proxyPort}${boot}` };
    case "degraded":
      return {
        status: "warn",
        detail: `proxy up on :${proxyPort} but a request through it failed (nodes down or starting)${boot}`,
      };
    case "down":
      return { status: "fail", detail: `nothing listening on :${proxyPort}${boot}` };
  }
}

// --- Clash config ---

export const CLASH_URL_PLACEHOLDER = "{{CLASH_SUBSCRIPTION_URL}}";

/**
 * Fill the subscription URL into the config template. The template holds it inside a
 * double-quoted YAML scalar, so it is escaped for that, and it is inserted through a
 * replacer FUNCTION: a plain string replacement would interpret `$&` and `$1` inside
 * the URL.
 */
export function renderClashConfig(template: string, subscriptionUrl: string): string {
  const escaped = JSON.stringify(subscriptionUrl).slice(1, -1);
  return template.replaceAll(CLASH_URL_PLACEHOLDER, () => escaped);
}

// --- timing policy of the start-up bring-up ---

export interface Wait {
  attempts: number;
  delayMs: number;
}

export interface EgressTiming {
  /** How long to wait for the proxy port to accept connections. */
  proxyWait: Wait;
  /** How long to wait for the subscription provider to load nodes. */
  providerWait: Wait;
  /**
   * After an explicit refresh. The refresh call returns once the fetch is done, so this only
   * covers the nodes becoming visible.
   */
  refreshWait: Wait;
  /** Pause after selecting the auto group, so the first connection can establish. */
  settleMs: number;
  /** The probe through the proxy. */
  probe: Wait;
}

export const DEFAULT_EGRESS_TIMING: EgressTiming = {
  proxyWait: { attempts: 30, delayMs: 1000 },
  providerWait: { attempts: 15, delayMs: 2000 },
  refreshWait: { attempts: 3, delayMs: 2000 },
  settleMs: 2000,
  probe: { attempts: 3, delayMs: 3000 },
};
