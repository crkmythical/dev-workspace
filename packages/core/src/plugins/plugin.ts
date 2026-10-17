/**
 * The plugin contract: what an optional feature may contribute to the core system.
 *
 * Deliberately tiny. A plugin is one folder per layer (here its pure parts, and its
 * commands under cli/src/plugins/<id>/) plus ONE line in the registry (./index.ts).
 * Everything that has to treat "all the encrypted volumes" alike (destroy, stale-mount
 * cleanup at boot, doctor) iterates the registry instead of naming a plugin, which is
 * what keeps the rest of the code free of any plugin's name (see R12 in
 * tests/architecture.test.ts).
 *
 * Grow it only when a second plugin needs the same thing, one capability at a time.
 */
import type { VaultSpec } from "../vault/domain.ts";
import type { EnvGroup } from "../workspace/env-schema.ts";

export interface Plugin {
  id: string;
  /**
   * Extra encrypted volumes the plugin owns. They are destroyed with the workspace,
   * detached when stale at boot, and reported by `doctor`.
   */
  vaults: readonly VaultSpec[];
  /** Environment variables the plugin reads: documented in the generated `.env.example`. */
  env?: readonly EnvGroup[];
}
