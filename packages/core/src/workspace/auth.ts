/**
 * The master password: ONE secret shared by code-server, the desktops and the vault.
 *
 * Where it comes from, and how it is kept in code-server's config file. Pure: the
 * caller reads the existing file, supplies a generator, and writes the result.
 */

export type PasswordSource =
  /** PASSWORD in the environment. */
  | "env"
  /** Nothing configured: a random one was made and must be shown to the user. */
  | "generated"
  /** Read back from an existing code-server config. */
  | "existing";

export interface MasterPassword {
  value: string;
  source: PasswordSource;
  /** The code-server config has to be (re)written for this password to take effect. */
  writeConfig: boolean;
}

export interface PasswordInput {
  /** PASSWORD from the environment, if set. */
  envPassword: string | undefined;
  /** Contents of code-server's config.yaml, or undefined when there is none yet. */
  existingConfig: string | undefined;
  generate: () => string;
}

/**
 * Precedence: an explicit PASSWORD always wins (and is written out); otherwise the
 * password already in code-server's config is kept, so restarts do not change it;
 * otherwise a new one is generated and written.
 */
export function resolveMasterPassword(input: PasswordInput): MasterPassword {
  const { envPassword, existingConfig, generate } = input;

  if (envPassword) {
    // The password goes onto one line of YAML, into a command line and into a prompt.
    if (/[\r\n]/.test(envPassword)) throw new Error("PASSWORD must be a single line");
    return { value: envPassword, source: "env", writeConfig: true };
  }

  const existing = existingConfig === undefined ? null : parseCodeServerPassword(existingConfig);
  if (existing) return { value: existing, source: "existing", writeConfig: false };

  // No config yet, or one without a usable password line: what we show must be what
  // code-server enforces, so write the new one out.
  return { value: generate(), source: "generated", writeConfig: true };
}

/**
 * code-server's config.yaml. The password is a double-quoted scalar (JSON string
 * syntax is valid YAML), so characters like `#` or `: ` cannot change its meaning.
 */
export function renderCodeServerConfig(password: string, port: number): string {
  return `bind-addr: 127.0.0.1:${port}\nauth: password\npassword: ${JSON.stringify(password)}\ncert: false\n`;
}

/** Read the password back; understands what render writes and the older unquoted form. */
export function parseCodeServerPassword(config: string): string | null {
  const m = config.match(/^password:[ \t]*(.+)$/m);
  if (!m) return null;
  const raw = m[1].trim();
  if (raw.startsWith('"')) {
    try {
      const parsed: unknown = JSON.parse(raw);
      return typeof parsed === "string" && parsed !== "" ? parsed : null;
    } catch {
      return null;
    }
  }
  if (raw.length >= 2 && raw.startsWith("'") && raw.endsWith("'")) {
    return raw.slice(1, -1).replaceAll("''", "'") || null;
  }
  return raw || null;
}
