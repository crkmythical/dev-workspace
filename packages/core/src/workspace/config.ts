/**
 * Typed view of the user-facing environment.
 *
 * This is the only place that knows the names and defaults of the variables the
 * TypeScript side reads. Composition roots (cli entry files, the server's
 * index.ts) call loadConfig(process.env) ONCE and hand the pieces they need to
 * adapters and routes, so library code never reaches into process.env itself
 * (enforced by tests/architecture.test.ts).
 *
 * Pure: it takes the environment as a parameter.
 */
import type { BackupGateEnv, EnvLike } from "../backup/domain.ts";
import { DESKTOP_DEFAULT_RESOLUTION } from "../desktop/layout.ts";
import { type FeatureFlags, parseFeatureFlags } from "./features.ts";

export interface WorkspaceConfig {
  /** Clash subscription URL. Empty when unset (see validateRequired). */
  clashSubscriptionUrl: string;
  /** Master password shared by code-server, the desktops and the vault. Generated when unset. */
  password: string | undefined;
  desktopResolution: string;
  /**
   * Host port docker-compose publishes the web UI on (TUNNEL_HOST_PORT). Only used to print
   * correct URLs: the container itself always listens on 8080.
   */
  hostPort: string;
  git: {
    userName: string | undefined;
    userEmail: string | undefined;
    /** Remote that vault ciphertext is replicated to (HTTPS+token or SSH URL). */
    vaultRepo: string | undefined;
  };
  /** restic / S3 variables, evaluated by evaluateBackupGate(). */
  backup: BackupGateEnv;
  features: FeatureFlags;
}

/** An unset variable and an empty one are the same thing. */
function value(env: EnvLike, name: string): string | undefined {
  const v = env[name];
  return v === undefined || v === "" ? undefined : v;
}

/** What docker-compose publishes when TUNNEL_HOST_PORT is unset. */
export const DEFAULT_HOST_PORT = "18080";

/** A port number as docker-compose would accept it; anything else would only mislead in a URL. */
function hostPortOf(env: EnvLike): string {
  const raw = value(env, "TUNNEL_HOST_PORT")?.trim();
  const port = raw !== undefined && /^\d{1,5}$/.test(raw) ? Number(raw) : 0;
  return port >= 1 && port <= 65535 ? String(port) : DEFAULT_HOST_PORT;
}

export function loadConfig(env: EnvLike): WorkspaceConfig {
  return {
    clashSubscriptionUrl: value(env, "CLASH_SUBSCRIPTION_URL") ?? "",
    password: value(env, "PASSWORD"),
    desktopResolution: value(env, "DESKTOP_RESOLUTION") ?? DESKTOP_DEFAULT_RESOLUTION,
    hostPort: hostPortOf(env),
    git: {
      userName: value(env, "GIT_USER_NAME"),
      userEmail: value(env, "GIT_USER_EMAIL"),
      vaultRepo: value(env, "VAULT_GIT_REPO"),
    },
    backup: {
      RESTIC_REPOSITORY: value(env, "RESTIC_REPOSITORY"),
      RESTIC_PASSWORD: value(env, "RESTIC_PASSWORD"),
      AWS_ACCESS_KEY_ID: value(env, "AWS_ACCESS_KEY_ID"),
      AWS_SECRET_ACCESS_KEY: value(env, "AWS_SECRET_ACCESS_KEY"),
    },
    features: parseFeatureFlags(env),
  };
}

// --- startup validation (defense in depth behind scripts/entrypoint.sh) ---

/** Variables that MUST be present for the container to start. */
export const REQUIRED_ENV_VARS = ["CLASH_SUBSCRIPTION_URL"] as const;

export type RequiredEnvVar = (typeof REQUIRED_ENV_VARS)[number];

/**
 * Return the list of required env vars that are missing (absent or empty).
 * A variable counts as present only when it has a non-empty value.
 */
export function findMissingRequiredEnv(
  env: EnvLike,
  required: readonly string[] = REQUIRED_ENV_VARS,
): string[] {
  return required.filter((name) => value(env, name) === undefined);
}

/**
 * Build the error message naming at least one missing variable.
 * Returns null when nothing is missing.
 */
export function envValidationError(missing: string[]): string | null {
  if (missing.length === 0) return null;
  return `ERROR: Required env var ${missing[0]} is not set.`;
}
