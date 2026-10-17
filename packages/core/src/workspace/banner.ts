/**
 * The summary printed at the end of a start-up: where to go, how to log in, and,
 * above all, what is NOT working. A degraded start used to be a warning somewhere in
 * the scroll-back; here it is part of the last thing on screen.
 */
import { type EgressReport, describeEgress } from "../egress/domain.ts";

export interface BannerInput {
  password: string;
  /** Host port the web UI is published on (config.hostPort). */
  hostPort: string;
  /** Which desktop stacks the image contains. */
  stacks: readonly ("selkies" | "vnc")[];
  vaultMounted: boolean;
  egress: EgressReport | undefined;
  /** Boot steps that failed under the "degrade" policy. */
  degraded: readonly string[];
}

const INNER_WIDTH = 50;

function box(rows: readonly string[]): string[] {
  const bar = "═".repeat(INNER_WIDTH);
  const row = (content: string) => `║${content.padEnd(INNER_WIDTH)}║`;
  const [title, ...body] = rows;
  return [`╔${bar}╗`, row(title), `╠${bar}╣`, ...body.map(row), `╚${bar}╝`];
}

/** Short status word for the box; the full sentence goes below it when something is off. */
function egressWord(egress: EgressReport | undefined): string {
  if (!egress) return "UNKNOWN";
  return egress.status === "confirmed" ? "confirmed" : egress.status.toUpperCase();
}

export function renderBanner(input: BannerInput): string[] {
  const { password, hostPort, stacks, vaultMounted, egress, degraded } = input;
  const origin = `http://localhost:${hostPort}`;

  const rows = [
    `${" ".repeat(10)}Secure Dev Workspace Ready`,
    `  Password:  ${password}`,
    `  Code:      ${origin}`,
    ...(stacks.includes("selkies") ? [`  Desktop:   ${origin}/desktop/`] : []),
    ...(stacks.includes("vnc") ? [`  VNC:       ${origin}/vnc/`] : []),
    `  Auth:      user / ${password}`,
    `  Vault:     ${vaultMounted ? "unlocked" : "LOCKED"}`,
    `  Egress:    ${egressWord(egress)}`,
  ];

  const attention: string[] = [];
  if (!vaultMounted) {
    attention.push("Vault is locked: run `unlock-vault` to open /workspace.");
  }
  if (!egress) {
    attention.push("Egress was not checked (the start-up step failed). Run `doctor`.");
  } else if (egress.status !== "confirmed") {
    attention.push(`Egress ${describeEgress(egress)}. Run \`doctor\`.`);
  }
  if (degraded.length > 0) {
    attention.push(`Start-up steps that failed (continuing without them): ${degraded.join(", ")}.`);
  }

  return [
    "",
    ...box(rows),
    ...(attention.length > 0 ? ["", ...attention.map((line) => `⚠ ${line}`)] : []),
    "",
  ];
}
