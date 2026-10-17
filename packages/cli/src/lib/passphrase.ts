/**
 * Passphrase input is a CLI concern (TTY / stdin), so it stays out of the
 * vault use cases, which receive a lazy reader instead.
 */

/** Read passphrase from env, TTY prompt, or piped stdin. */
export async function readPassphrase(envVar?: string, promptMsg = "Passphrase: "): Promise<string> {
  if (envVar && process.env[envVar]) {
    return process.env[envVar]!;
  }
  if (process.stdin.isTTY) {
    return prompt(promptMsg) ?? "";
  }
  return (await new Response(process.stdin).text()).trim();
}
