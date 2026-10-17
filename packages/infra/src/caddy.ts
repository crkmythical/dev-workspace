/** Caddy helpers. */

/**
 * bcrypt hash of a password in the form Caddy's `basic_auth` expects (it guards the
 * desktops). Empty when `caddy` cannot produce one; the caller decides what that means.
 */
export async function caddyPasswordHash(password: string): Promise<string> {
  try {
    const proc = Bun.spawn(["caddy", "hash-password", "--plaintext", password], {
      stdout: "pipe",
      stderr: "pipe",
    });
    const out = await new Response(proc.stdout).text();
    await proc.exited;
    return out.trim();
  } catch {
    return "";
  }
}
