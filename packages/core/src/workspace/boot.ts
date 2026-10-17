/**
 * The container's start-up as DATA: which steps run, in which order, what a failure of
 * each one means, and which ones may only run once the vault is mounted.
 *
 * Writing this down once (and testing it) is the point. It used to be implicit in a
 * 250-line top-level script, where an unexpected throw anywhere crashed the container
 * into a restart loop and the only record of a degraded start was a log line.
 *
 *   abort    a failure stops the boot: running on would mean running without a
 *            password, or not running at all.
 *   degrade  a failure is recorded and the boot goes on: the workspace is still
 *            worth having (no proxy, no desktop, no backup, ...) and the banner says
 *            what is missing.
 *
 * The effects of each step live in the composition root (cli/entrypoint-main.ts),
 * which must supply one implementation per id; the compiler enforces that.
 */
import type { EgressReport } from "../egress/domain.ts";

export interface BootState {
  /**
   * The workspace vault is mounted. Anything that writes under the mount point while
   * this is false would land in the container's own, UNencrypted filesystem.
   */
  vaultMounted: boolean;
  /** What bring-up-egress observed. */
  egress?: EgressReport;
  /** Ids of steps that failed under the "degrade" policy. */
  degraded: string[];
}

export interface BootStepSpec {
  id: string;
  onFail: "abort" | "degrade";
  /** The step is skipped (not failed) while this is false. */
  when?: (state: BootState) => boolean;
}

const vaultIsMounted = (s: BootState) => s.vaultMounted;

export const BOOT_PLAN = [
  // Required environment is present; DESKTOP_RESOLUTION is defaulted for supervisord.
  { id: "preflight", onFail: "abort" },
  // A mount left over from the previous container lifecycle would fake an unlocked state.
  { id: "cleanup-stale-mounts", onFail: "degrade" },
  // Clash needs its config before it starts.
  { id: "render-clash-config", onFail: "degrade" },
  { id: "git-globals", onFail: "degrade" },
  { id: "tune-inotify", onFail: "degrade" },
  // Start the proxy and observe whether traffic gets out. Best effort: a proxy that is
  // down is reported, never a reason to stop.
  { id: "bring-up-egress", onFail: "degrade" },
  // The master password is also the vault passphrase: it must exist before the vault.
  { id: "resolve-auth", onFail: "abort" },
  { id: "init-vault", onFail: "degrade" },
  { id: "unlock-vault", onFail: "degrade" },
  // Everything below writes under the mount point, so it needs the vault.
  { id: "notify-egress", onFail: "degrade", when: vaultIsMounted },
  { id: "prepare-desktops", onFail: "degrade", when: vaultIsMounted },
  { id: "prepare-backup", onFail: "degrade", when: vaultIsMounted },
  // The summary needs the final state of everything above.
  { id: "banner", onFail: "degrade" },
  // Hands over to supervisord, which runs until the container stops.
  { id: "supervisord", onFail: "abort" },
] as const satisfies readonly BootStepSpec[];

export type BootStepId = (typeof BOOT_PLAN)[number]["id"];

export type BootStepImpls = Record<BootStepId, (state: BootState) => Promise<void>>;

export type StepOutcome = "ok" | "skipped" | "degraded" | "aborted";

export interface StepReport {
  id: BootStepId;
  outcome: StepOutcome;
  /** The error message, for "degraded" and "aborted". */
  detail?: string;
}

export interface BootReport {
  steps: StepReport[];
  /** An abort-policy step failed; the later steps did not run. */
  aborted: boolean;
}

export async function runBoot(
  plan: readonly (BootStepSpec & { id: BootStepId })[],
  impls: BootStepImpls,
  state: BootState,
  log: (message: string) => void,
): Promise<BootReport> {
  const steps: StepReport[] = [];
  for (const step of plan) {
    if (step.when && !step.when(state)) {
      steps.push({ id: step.id, outcome: "skipped" });
      continue;
    }
    try {
      await impls[step.id](state);
      steps.push({ id: step.id, outcome: "ok" });
    } catch (err) {
      const detail = err instanceof Error ? err.message : String(err);
      if (step.onFail === "abort") {
        steps.push({ id: step.id, outcome: "aborted", detail });
        log(`ERROR: boot aborted at "${step.id}": ${detail}`);
        return { steps, aborted: true };
      }
      state.degraded.push(step.id);
      steps.push({ id: step.id, outcome: "degraded", detail });
      log(`WARNING: boot step "${step.id}" failed, continuing without it: ${detail}`);
    }
  }
  return { steps, aborted: false };
}
