/**
 * HolderControl adapter: stops everything that may hold fds or inotify watches
 * on the plaintext mount. Desktops first (XFCE is the primary HOME-fd holder),
 * then the backup watcher.
 */
import type { HolderControl } from "@sdw/core/vault";
import { $ } from "bun";
import { stopAllDesktops } from "./desktop.ts";

export const supervisorHolders: HolderControl = {
  async stopAll() {
    await stopAllDesktops();
    await $`supervisorctl stop backup-watcher`.quiet().nothrow();
  },
};
