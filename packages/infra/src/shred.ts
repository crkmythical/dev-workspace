/**
 * Shredder adapter. shred(1) overwrites the file; if that fails for any reason
 * the file is still unlinked, so a key file is never left in place just because
 * the overwrite step errored. A missing file counts as already gone.
 */
import { existsSync } from "node:fs";
import type { Shredder } from "@sdw/core/vault";
import { $ } from "bun";

export const fileShredder: Shredder = {
  async shred(path, passes) {
    if (!existsSync(path)) return true;
    await $`shred -n${passes} -z ${path}`.quiet().nothrow();
    await $`rm -f ${path}`.quiet().nothrow();
    return !existsSync(path);
  },
};
