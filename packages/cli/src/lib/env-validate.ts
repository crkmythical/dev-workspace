/**
 * Pure environment-variable validation for container startup.
 *
 * Mirrors the required-variable check in scripts/entrypoint.sh so the
 * completeness invariant (Property 18) is testable without spawning a
 * container. entrypoint-main.ts calls findMissingRequiredEnv as a
 * defense-in-depth re-check after the bash pre-flight.
 */

/** Variables that MUST be present for the container to start. */
export const REQUIRED_ENV_VARS = ["CLASH_SUBSCRIPTION_URL"] as const;

export type RequiredEnvVar = (typeof REQUIRED_ENV_VARS)[number];

/**
 * Return the list of required env vars that are missing (absent or empty).
 * A variable counts as present only when it has a non-empty value.
 */
export function findMissingRequiredEnv(
  env: Record<string, string | undefined>,
  required: readonly string[] = REQUIRED_ENV_VARS,
): string[] {
  return required.filter((name) => {
    const value = env[name];
    return value === undefined || value === "";
  });
}

/**
 * Build the error message naming at least one missing variable.
 * Returns null when nothing is missing.
 */
export function envValidationError(missing: string[]): string | null {
  if (missing.length === 0) return null;
  return `ERROR: Required env var ${missing[0]} is not set.`;
}
