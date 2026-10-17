/**
 * Passphrase input is a CLI concern (TTY / stdin), so it stays out of the
 * vault use cases, which receive a lazy reader instead.
 *
 * `preset` is a value the caller already took from its configuration (for
 * example the PASSWORD variable); this module never reads the environment.
 */

/** Use `preset` when given, else prompt on a TTY, else read piped stdin. */
export async function readPassphrase(preset?: string, promptMsg = "Passphrase: "): Promise<string> {
  if (preset) return preset;
  if (process.stdin.isTTY) {
    return prompt(promptMsg) ?? "";
  }
  return (await new Response(process.stdin).text()).trim();
}
