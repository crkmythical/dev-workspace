/**
 * FailureTracker on plain files, replacing the two copies that vault-sync and the
 * backup watcher each carried.
 *
 *   <stateDir>/last-success            ISO timestamp of the last success
 *   <stateDir>/consecutive-failures    the counter (what `doctor` reads)
 *   <notificationsDir>/<file>          written once the threshold is reached
 *
 * State is container-local (non-FUSE); losing it on restart is harmless.
 */
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { type FailureTracker, reachedFailureThreshold } from "@sdw/core/shared/failure";
import { NOTIFICATIONS_DIR } from "@sdw/core/shared/layout";
import { removeNote, writeNote } from "./notifications.ts";

export interface FailureTrackerOptions {
  stateDir: string;
  threshold: number;
  /** File name inside the notifications directory, e.g. "backup-failure.md". */
  notificationFile: string;
  /** Markdown body for the notification, given the failure count. */
  notificationBody: (count: number) => string;
  /** Where notifications go; defaults to the vault's .notifications directory. */
  notificationsDir?: string;
}

export function createFileFailureTracker(opts: FailureTrackerOptions): FailureTracker {
  const countFile = path.join(opts.stateDir, "consecutive-failures");
  const successFile = path.join(opts.stateDir, "last-success");
  const notificationsDir = opts.notificationsDir ?? NOTIFICATIONS_DIR;
  mkdirSync(opts.stateDir, { recursive: true });

  const read = (): number => {
    try {
      return Number.parseInt(readFileSync(countFile, "utf-8").trim(), 10) || 0;
    } catch {
      return 0;
    }
  };

  return {
    consecutiveFailures: read,

    recordSuccess() {
      writeFileSync(successFile, new Date().toISOString());
      writeFileSync(countFile, "0");
      // A job that works again must not keep telling the user it is failing (`doctor` is
      // already green; the note used to stay until someone deleted it).
      removeNote(path.join(notificationsDir, opts.notificationFile));
    },

    recordFailure() {
      const count = read() + 1;
      writeFileSync(countFile, String(count));
      if (!reachedFailureThreshold(count, opts.threshold)) return { count, notified: false };
      // With the vault locked there is nowhere safe to write; that is not fatal.
      const notified = writeNote(
        path.join(notificationsDir, opts.notificationFile),
        opts.notificationBody(count),
      );
      return { count, notified };
    },
  };
}
