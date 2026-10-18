/**
 * HolderControl adapter: stops everything that may hold fds or inotify watches
 * on the plaintext mount. Desktops first (XFCE is the primary HOME-fd holder),
 * then the backup watcher.
 */
import type { DesktopTarget } from "@sdw/core/desktop";
import type { HolderControl } from "@sdw/core/vault";
import { $ } from "bun";
import { installedStacks, isDesktopRunning, startDesktop, stopAllDesktops } from "./desktop.ts";

async function backupWatcherRunning(): Promise<boolean> {
  const r = await $`supervisorctl status backup-watcher`.quiet().nothrow();
  return r.text().includes("RUNNING");
}

/** What the first `stopAll` of a lock attempt found running; kept for `restore`. */
let stoppedByUs: { desktops: DesktopTarget[]; backup: boolean } | undefined;

export const supervisorHolders: HolderControl = {
  async stopAll() {
    // The second call of a forced retry sees everything stopped already; the first one knows.
    if (!stoppedByUs) {
      const desktops: DesktopTarget[] = [];
      for (const target of installedStacks()) {
        if (await isDesktopRunning(target)) desktops.push(target);
      }
      stoppedByUs = { desktops, backup: await backupWatcherRunning() };
    }
    await stopAllDesktops();
    await $`supervisorctl stop backup-watcher`.quiet().nothrow();
  },

  async restore() {
    const was = stoppedByUs;
    stoppedByUs = undefined;
    if (!was) return [];
    const started: string[] = [];
    if (was.backup) {
      await $`supervisorctl start backup-watcher`.quiet().nothrow();
      started.push("backup watcher");
    }
    for (const target of was.desktops) {
      await startDesktop(target);
      started.push(`desktop [${target}]`);
    }
    return started;
  },
};
