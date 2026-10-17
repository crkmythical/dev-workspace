/**
 * Plugin registry: the optional features compiled into this build.
 *
 * To drop a plugin, delete its line here and its folders (core/src/plugins/<id>,
 * cli/src/plugins/<id>). Nothing else in the source names it.
 */
import type { VaultSpec } from "../vault/domain.ts";
import { WORKSPACE_VAULT } from "../vault/specs.ts";
import { CORE_ENV_GROUPS, type EnvGroup } from "../workspace/env-schema.ts";
import { pentestPlugin } from "./pentest/index.ts";
import type { Plugin } from "./plugin.ts";

export type { Plugin } from "./plugin.ts";

export const PLUGINS: readonly Plugin[] = [pentestPlugin];

/** Every variable the user can set: the core's, then each plugin's (generates `.env.example`). */
export const ENV_GROUPS: readonly EnvGroup[] = [
  ...CORE_ENV_GROUPS,
  ...PLUGINS.flatMap((plugin) => plugin.env ?? []),
];

/** Every encrypted volume of this build: the workspace first, then each plugin's. */
export const ALL_VAULTS: readonly VaultSpec[] = [
  WORKSPACE_VAULT,
  ...PLUGINS.flatMap((plugin) => plugin.vaults),
];
